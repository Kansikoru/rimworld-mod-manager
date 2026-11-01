using System;
using System.Diagnostics;
using System.Globalization;
using Verse;

namespace RimWorldModManagerCompanion
{
    /// <summary>
    /// Writes the loading time to Player.log once the game has finished loading, for
    /// RimWorld Mod Manager. Static constructors with this attribute run at the very end of
    /// loading; the line is written after the rest of that work (queued behind it).
    /// </summary>
    [StaticConstructorOnStartup]
    public static class LoadTimeReporter
    {
        /// <summary>Read by the manager: keep the wording.</summary>
        public const string Marker = "[RimWorld Mod Manager Companion] Loading finished in ";

        static LoadTimeReporter()
        {
            LongEventHandler.ExecuteWhenFinished(Report);
        }

        private static void Report()
        {
            try
            {
                // From the start of the game process: what the player waits for.
                var started = Process.GetCurrentProcess().StartTime;
                var secs = (DateTime.Now - started).TotalSeconds;
                Log.Message(Marker + secs.ToString("F2", CultureInfo.InvariantCulture) + " s");
            }
            catch (Exception e)
            {
                Log.Warning("[RimWorld Mod Manager Companion] Could not measure the loading time: " + e.Message);
            }
        }
    }
}
