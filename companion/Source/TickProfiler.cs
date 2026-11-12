using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text;
using RimWorld;
using Verse;

namespace RimWorldModManagerCompanion
{
    /// <summary>
    /// When RimWorld Mod Manager asks for it (a request file, used once), measures how much of
    /// a game tick the mods' own ticking code takes: every Tick / CompTick / ...ComponentTick
    /// method declared in a mod's assembly gets a timer around it (through Harmony, found by
    /// reflection). After the asked number of ticks the result is written to
    /// %LOCALAPPDATA%\RimWorldModManager\tick_profile.json and the timers go quiet.
    /// Without a request nothing here runs. What mods do inside their Harmony patches of the
    /// game's own methods is not measured.
    /// </summary>
    public static class TickProfiler
    {
        private const int Format = 1;
        private const string HarmonyId = "kansikoru.rmm.companion.ticks";
        private const string Tag = "[RimWorld Mod Manager Companion] ";
        /// <summary>Patching takes a few milliseconds per method: a ceiling keeps the start short.</summary>
        private const int MaxMethods = 3000;

        private static readonly HashSet<string> TickMethods = new HashSet<string>
        {
            "Tick", "TickRare", "TickLong", "TickInterval",
            "CompTick", "CompTickRare", "CompTickLong", "CompTickInterval",
            "MapComponentTick", "GameComponentTick", "WorldComponentTick",
            "MapComponentUpdate", "GameComponentUpdate", "WorldComponentUpdate",
        };

        private sealed class Slot
        {
            public int Mod;
            public string Name;
            public long Time;
            public long Calls;
        }

        private static readonly Dictionary<MethodBase, Slot> Slots = new Dictionary<MethodBase, Slot>();
        private static readonly List<string> Errors = new List<string>();
        private static List<ModContentPack> mods;
        private static volatile bool active;
        private static long wantTicks;
        private static long ticks;
        private static long tickTime;
        private static int skipped;

        private static string Folder =>
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "RimWorldModManager");

        /// <summary>Called once the game has loaded. The request file holds the number of ticks.</summary>
        public static void StartIfAsked()
        {
            string request = Path.Combine(Folder, "tick_profile.request");
            if (!File.Exists(request)) return;
            long want = 7200;
            try
            {
                if (long.TryParse(File.ReadAllText(request).Trim(), NumberStyles.Integer, CultureInfo.InvariantCulture, out long asked))
                {
                    want = Math.Max(600, Math.Min(asked, 216000));
                }
                File.Delete(request);
            }
            catch (Exception e)
            {
                // A request that can't be removed would measure on every launch.
                Log.Warning(Tag + "Could not take the tick measuring request: " + e.Message);
                return;
            }

            Type harmony = CompanionReport.FindHarmony();
            if (harmony == null)
            {
                Log.Warning(Tag + "Tick measuring needs Harmony, and it is not loaded.");
                return;
            }
            try
            {
                mods = LoadedModManager.RunningModsListForReading.ToList();
                int patched = Patch(harmony);
                wantTicks = want;
                active = true;
                Log.Message(Tag + "Measuring the mods' tick time: " + patched + " methods, " + want + " ticks.");
            }
            catch (Exception e)
            {
                Log.Warning(Tag + "Could not start tick measuring: " + e.GetType().Name + ": " + e.Message);
            }
        }

        private static int Patch(Type harmony)
        {
            object instance = Activator.CreateInstance(harmony, new object[] { HarmonyId });
            Type methodType = harmony.Assembly.GetType("HarmonyLib.HarmonyMethod", true);
            ConstructorInfo wrap = methodType.GetConstructor(new[] { typeof(MethodInfo) });
            // Patch(MethodBase original, HarmonyMethod prefix, HarmonyMethod postfix, ...): the
            // number of trailing parameters differs between Harmony versions.
            MethodInfo patch = harmony.GetMethods(BindingFlags.Public | BindingFlags.Instance).FirstOrDefault(m =>
            {
                ParameterInfo[] p = m.GetParameters();
                return m.Name == "Patch" && p.Length >= 3 && p[0].ParameterType == typeof(MethodBase)
                    && p.Skip(1).All(x => x.ParameterType == methodType);
            });
            if (wrap == null || patch == null) throw new MissingMethodException("Harmony.Patch");
            int parameters = patch.GetParameters().Length;

            Action<MethodBase, string, string> apply = (original, before, after) =>
            {
                var args = new object[parameters];
                args[0] = original;
                args[1] = wrap.Invoke(new object[] { typeof(TickProfiler).GetMethod(before) });
                args[2] = wrap.Invoke(new object[] { typeof(TickProfiler).GetMethod(after) });
                patch.Invoke(instance, args);
            };

            // The whole tick, to say what share of it the mods take.
            MethodInfo singleTick = typeof(TickManager).GetMethod("DoSingleTick", BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance);
            if (singleTick == null) throw new MissingMethodException("TickManager.DoSingleTick");
            apply(singleTick, nameof(TickBefore), nameof(TickAfter));

            Assembly own = typeof(TickProfiler).Assembly;
            const BindingFlags declared = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.DeclaredOnly;
            for (int i = 0; i < mods.Count; i++)
            {
                if (mods[i].IsOfficialMod) continue;
                List<Assembly> assemblies = mods[i].assemblies?.loadedAssemblies;
                if (assemblies == null) continue;
                foreach (Assembly assembly in assemblies)
                {
                    if (assembly == null || assembly == own) continue;
                    foreach (Type type in TypesOf(assembly))
                    {
                        if (type == null || type.ContainsGenericParameters) continue;
                        MethodInfo[] methods;
                        try
                        {
                            methods = type.GetMethods(declared);
                        }
                        catch (Exception)
                        {
                            continue;
                        }
                        foreach (MethodInfo method in methods)
                        {
                            if (!TickMethods.Contains(method.Name) || method.IsAbstract || method.IsGenericMethod) continue;
                            if (Slots.ContainsKey(method)) continue;
                            if (Slots.Count >= MaxMethods)
                            {
                                skipped++;
                                continue;
                            }
                            try
                            {
                                if (method.GetMethodBody() == null) continue;
                                apply(method, nameof(Before), nameof(After));
                                Slots[method] = new Slot { Mod = i, Name = type.FullName + "." + method.Name };
                            }
                            catch (Exception e)
                            {
                                if (Errors.Count < 5)
                                {
                                    Exception cause = e.InnerException ?? e;
                                    Errors.Add(type.FullName + "." + method.Name + ": " + cause.GetType().Name + ": " + cause.Message);
                                }
                            }
                        }
                    }
                }
            }
            return Slots.Count;
        }

        private static Type[] TypesOf(Assembly assembly)
        {
            try
            {
                return assembly.GetTypes();
            }
            catch (ReflectionTypeLoadException e)
            {
                return e.Types ?? new Type[0];
            }
            catch (Exception)
            {
                return new Type[0];
            }
        }

        public static void Before(out long __state)
        {
            __state = active ? Stopwatch.GetTimestamp() : 0;
        }

        public static void After(long __state, MethodBase __originalMethod)
        {
            if (__state == 0 || !active) return;
            if (Slots.TryGetValue(__originalMethod, out Slot slot))
            {
                slot.Time += Stopwatch.GetTimestamp() - __state;
                slot.Calls++;
            }
        }

        public static void TickBefore(out long __state)
        {
            __state = active ? Stopwatch.GetTimestamp() : 0;
        }

        public static void TickAfter(long __state)
        {
            if (__state == 0 || !active) return;
            tickTime += Stopwatch.GetTimestamp() - __state;
            ticks++;
            if (ticks < wantTicks) return;
            active = false;
            try
            {
                Write();
                Log.Message(Tag + "Tick measuring finished: " + ticks + " ticks.");
            }
            catch (Exception e)
            {
                Log.Warning(Tag + "Could not write the tick measurement: " + e.Message);
            }
        }

        private static void Write()
        {
            Directory.CreateDirectory(Folder);
            string path = Path.Combine(Folder, "tick_profile.json");
            string tmp = path + ".tmp";
            File.WriteAllText(tmp, Build(), new UTF8Encoding(false));
            if (File.Exists(path)) File.Delete(path);
            File.Move(tmp, path);
        }

        private static string Build()
        {
            double perTick = 1000.0 / Stopwatch.Frequency / Math.Max(1, ticks);
            var json = new JsonWriter();
            json.BeginObject();
            json.Field("format", Format);
            json.Field("game", VersionControl.CurrentVersionStringWithRev);
            json.Field("written_unix", DateTimeOffset.UtcNow.ToUnixTimeSeconds());
            json.Field("ticks", ticks);
            json.Field("tick_ms", tickTime * perTick);
            json.Field("patched_methods", Slots.Count);
            json.Field("skipped_methods", skipped);
            json.Name("mods");
            json.BeginArray();
            foreach (IGrouping<int, Slot> group in Slots.Values.Where(s => s.Calls > 0).GroupBy(s => s.Mod).OrderByDescending(g => g.Sum(s => s.Time)))
            {
                ModContentPack mod = mods[group.Key];
                json.BeginObject();
                json.Field("id", mod.PackageId);
                json.Field("name", mod.Name);
                json.Field("ms", group.Sum(s => s.Time) * perTick);
                json.Field("calls", group.Sum(s => s.Calls) / (double)Math.Max(1, ticks));
                json.Name("methods");
                json.BeginArray();
                foreach (Slot slot in group.OrderByDescending(s => s.Time).Take(5))
                {
                    json.BeginObject();
                    json.Field("name", slot.Name);
                    json.Field("ms", slot.Time * perTick);
                    json.Field("calls", slot.Calls / (double)Math.Max(1, ticks));
                    json.EndObject();
                }
                json.EndArray();
                json.EndObject();
            }
            json.EndArray();
            json.Name("errors");
            json.BeginArray();
            foreach (string e in Errors) json.Value(e);
            json.EndArray();
            json.EndObject();
            return json.ToString();
        }
    }
}
