using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text;
using RimWorld;
using Verse;

namespace RimWorldModManagerCompanion
{
    /// <summary>
    /// After loading, writes what only the game knows for RimWorld Mod Manager:
    /// texture memory per mod (1.6), defs one mod replaced in another, and methods that
    /// several mods patch with Harmony. Only reads the game; changes nothing.
    /// File: %LOCALAPPDATA%\RimWorldModManager\companion_report.json.
    /// </summary>
    internal static class CompanionReport
    {
        /// <summary>Bump when the manager has to tell the file versions apart.</summary>
        private const int Format = 1;
        /// <summary>Harmony's lists of patches and the kind the report names them by.</summary>
        private static readonly KeyValuePair<string, string>[] PatchKinds =
        {
            new KeyValuePair<string, string>("Prefixes", "prefix"),
            new KeyValuePair<string, string>("Postfixes", "postfix"),
            new KeyValuePair<string, string>("Transpilers", "transpiler"),
            new KeyValuePair<string, string>("Finalizers", "finalizer"),
        };

        public static void Write(double loadSecs)
        {
            string dir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "RimWorldModManager");
            Directory.CreateDirectory(dir);
            string path = Path.Combine(dir, "companion_report.json");
            // Through a temporary file: the manager never reads half a report.
            string tmp = path + ".tmp";
            File.WriteAllText(tmp, Build(loadSecs), new UTF8Encoding(false));
            if (File.Exists(path)) File.Delete(path);
            File.Move(tmp, path);
        }

        public static string Build(double loadSecs)
        {
            var watch = Stopwatch.StartNew();
            List<ModContentPack> mods = LoadedModManager.RunningModsListForReading;
            var errors = new List<string>();

            var json = new JsonWriter();
            json.BeginObject();
            json.Field("format", Format);
            json.Name("game");
            json.Raw(Section("game", errors, GameVersion));
            json.Field("written_unix", DateTimeOffset.UtcNow.ToUnixTimeSeconds());
            json.Field("load_secs", loadSecs);
            json.Name("mods");
            json.Raw(Section("mods", errors, () => ModList(mods)));
            json.Name("textures");
            json.Raw(Section("textures", errors, () => TextureMemory.Json(mods)));
            json.Name("def_overrides");
            json.Raw(Section("def_overrides", errors, () => DefOverrides(mods)));
            json.Name("harmony");
            json.Raw(Section("harmony", errors, () => SharedPatches(mods)));
            json.Name("errors");
            json.BeginArray();
            foreach (string e in errors) json.Value(e);
            json.EndArray();
            json.Field("report_ms", watch.ElapsedMilliseconds);
            json.EndObject();
            return json.ToString();
        }

        /// <summary>
        /// One part of the report; a failing part becomes null and an error line. Each part is
        /// its own method, so a game member missing in some build fails only that part.
        /// </summary>
        private static string Section(string name, List<string> errors, Func<string> build)
        {
            try
            {
                return build();
            }
            catch (Exception e)
            {
                errors.Add(name + ": " + e.GetType().Name + ": " + e.Message);
                return null;
            }
        }

        private static string GameVersion()
        {
            var json = new JsonWriter();
            json.Value(VersionControl.CurrentVersionStringWithRev);
            return json.ToString();
        }

        private static string ModList(List<ModContentPack> mods)
        {
            var json = new JsonWriter();
            json.BeginArray();
            foreach (ModContentPack mod in mods)
            {
                json.BeginObject();
                json.Field("id", mod.PackageId);
                json.Field("name", mod.Name);
                json.Field("official", mod.IsOfficialMod);
                json.EndObject();
            }
            json.EndArray();
            return json.ToString();
        }

        /// <summary>
        /// Defs that several mods define under one name. The game keeps the last one: it removes
        /// the older def from the database, but the def stays in its own mod's list.
        /// </summary>
        private static string DefOverrides(List<ModContentPack> mods)
        {
            var databases = new HashSet<Type>(GenDefDatabase.AllDefTypesWithDatabases());
            // "database type/defName" -> the mods (indices in load order) that define it.
            var definers = new Dictionary<string, List<int>>();
            var keys = new Dictionary<string, KeyValuePair<Type, string>>();
            for (int i = 0; i < mods.Count; i++)
            {
                foreach (Def def in mods[i].AllDefs)
                {
                    if (def == null || string.IsNullOrEmpty(def.defName)) continue;
                    Type database = DatabaseOf(def.GetType(), databases);
                    if (database == null) continue;
                    string key = database.FullName + "/" + def.defName;
                    if (!definers.TryGetValue(key, out List<int> list))
                    {
                        list = new List<int>();
                        definers[key] = list;
                        keys[key] = new KeyValuePair<Type, string>(database, def.defName);
                    }
                    if (!list.Contains(i)) list.Add(i);
                }
            }

            var index = new Dictionary<ModContentPack, int>();
            for (int i = 0; i < mods.Count; i++) index[mods[i]] = i;

            var json = new JsonWriter();
            json.BeginArray();
            foreach (KeyValuePair<string, List<int>> pair in definers)
            {
                if (pair.Value.Count < 2) continue;
                KeyValuePair<Type, string> key = keys[pair.Key];
                Def winner = GenDefDatabase.GetDefSilentFail(key.Key, key.Value, false);
                int winnerIndex = winner?.modContentPack != null && index.TryGetValue(winner.modContentPack, out int w) ? w : -1;
                json.BeginObject();
                json.Field("type", key.Key.Name);
                json.Field("def", key.Value);
                json.Name("mods");
                json.BeginArray();
                foreach (int m in pair.Value) json.Value(m);
                json.EndArray();
                json.Field("winner", winnerIndex);
                json.EndObject();
            }
            json.EndArray();
            return json.ToString();
        }

        /// <summary>The def type whose database holds defs of this type (a def class may derive from it).</summary>
        private static Type DatabaseOf(Type type, HashSet<Type> databases)
        {
            for (Type t = type; t != null && t != typeof(Def); t = t.BaseType)
            {
                if (databases.Contains(t)) return t;
            }
            return null;
        }

        /// <summary>
        /// Methods that patches of two or more mods change. Harmony is found by reflection:
        /// the companion needs no Harmony and works with 2.x (1.6) and 3.x (1.7).
        /// </summary>
        private static string SharedPatches(List<ModContentPack> mods)
        {
            Type harmony = FindHarmony();
            if (harmony == null) return null;
            MethodInfo getAll = harmony.GetMethod("GetAllPatchedMethods", BindingFlags.Public | BindingFlags.Static, null, Type.EmptyTypes, null);
            MethodInfo getInfo = harmony.GetMethod("GetPatchInfo", BindingFlags.Public | BindingFlags.Static, null, new[] { typeof(MethodBase) }, null);
            if (getAll == null || getInfo == null) return null;

            var modOf = new Dictionary<Assembly, int>();
            for (int i = 0; i < mods.Count; i++)
            {
                List<Assembly> loaded = mods[i].assemblies?.loadedAssemblies;
                if (loaded == null) continue;
                foreach (Assembly a in loaded)
                {
                    if (a != null && !modOf.ContainsKey(a)) modOf[a] = i;
                }
            }

            var json = new JsonWriter();
            json.BeginObject();
            json.Field("version", harmony.Assembly.GetName().Version?.ToString());
            int patched = 0;
            json.Name("shared");
            json.BeginArray();
            foreach (object o in (IEnumerable)getAll.Invoke(null, null))
            {
                if (!(o is MethodBase method)) continue;
                patched++;
                List<PatchRef> patches;
                try
                {
                    patches = PatchesOf(getInfo.Invoke(null, new object[] { method }), modOf);
                }
                catch (Exception)
                {
                    continue;
                }
                if (patches.Select(p => p.ModKey).Distinct().Count() < 2) continue;
                json.BeginObject();
                json.Field("method", Describe(method));
                json.Name("patches");
                json.BeginArray();
                foreach (PatchRef p in patches)
                {
                    json.BeginObject();
                    json.Field("mod", p.Mod);
                    json.Field("owner", p.Owner);
                    json.Field("kind", p.Kind);
                    json.Field("skips", p.Skips);
                    json.EndObject();
                }
                json.EndArray();
                json.EndObject();
            }
            json.EndArray();
            json.Field("patched_methods", patched);
            json.EndObject();
            return json.ToString();
        }

        private struct PatchRef
        {
            public int Mod;
            public string Owner;
            public string Kind;
            /// <summary>A prefix returning bool: it can stop the original and the patches after it.</summary>
            public bool Skips;
            public string ModKey => Mod >= 0 ? "#" + Mod : Owner ?? "";
        }

        private static List<PatchRef> PatchesOf(object info, Dictionary<Assembly, int> modOf)
        {
            var result = new List<PatchRef>();
            if (info == null) return result;
            foreach (KeyValuePair<string, string> kind in PatchKinds)
            {
                if (!(Member(info, kind.Key) is IEnumerable list)) continue;
                foreach (object patch in list)
                {
                    if (patch == null) continue;
                    var patchMethod = Member(patch, "PatchMethod") as MethodInfo;
                    Assembly assembly = patchMethod?.DeclaringType?.Assembly;
                    int mod = assembly != null && modOf.TryGetValue(assembly, out int m) ? m : -1;
                    result.Add(new PatchRef
                    {
                        Mod = mod,
                        Owner = Member(patch, "owner") as string,
                        Kind = kind.Value,
                        Skips = kind.Value == "prefix" && patchMethod != null && patchMethod.ReturnType == typeof(bool),
                    });
                }
            }
            return result;
        }

        private static Type FindHarmony()
        {
            foreach (Assembly a in AppDomain.CurrentDomain.GetAssemblies())
            {
                Type t;
                try
                {
                    t = a.GetType("HarmonyLib.Harmony", false);
                }
                catch (Exception)
                {
                    continue;
                }
                if (t != null && t.GetMethod("GetAllPatchedMethods", BindingFlags.Public | BindingFlags.Static) != null) return t;
            }
            return null;
        }

        /// <summary>A public field or property, whatever its case (Harmony versions differ).</summary>
        private static object Member(object target, string name)
        {
            const BindingFlags flags = BindingFlags.Public | BindingFlags.Instance | BindingFlags.IgnoreCase;
            Type type = target.GetType();
            FieldInfo field = type.GetField(name, flags);
            if (field != null) return field.GetValue(target);
            PropertyInfo property = type.GetProperty(name, flags);
            return property != null && property.GetIndexParameters().Length == 0 ? property.GetValue(target, null) : null;
        }

        private static string Describe(MethodBase method)
        {
            string parameters = string.Join(", ", method.GetParameters().Select(p => p.ParameterType.Name).ToArray());
            return (method.DeclaringType?.FullName ?? "?") + "." + method.Name + "(" + parameters + ")";
        }
    }
}
