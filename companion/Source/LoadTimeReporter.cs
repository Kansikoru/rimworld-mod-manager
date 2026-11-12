using System;
using System.Diagnostics;
using System.Globalization;
using Verse;

namespace RimWorldModManagerCompanion
{
    /// <summary>
    /// Writes the loading time to Player.log once the game has finished loading, then the
    /// report (CompanionReport), for RimWorld Mod Manager. Static constructors with this
    /// attribute run at the very end of loading; the work is queued behind the rest of it.
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
            double secs;
            try
            {
                // From the start of the game process: what the player waits for.
                var started = Process.GetCurrentProcess().StartTime;
                secs = (DateTime.Now - started).TotalSeconds;
                Log.Message(Marker + secs.ToString("F2", CultureInfo.InvariantCulture) + " s");
            }
            catch (Exception e)
            {
                Log.Warning("[RimWorld Mod Manager Companion] Could not measure the loading time: " + e.Message);
                secs = double.NaN;
            }
            try
            {
                CompanionReport.Write(secs);
            }
            catch (Exception e)
            {
                Log.Warning("[RimWorld Mod Manager Companion] Could not write the report: " + e.Message);
            }
            // Only when the manager asked for it; says so in the log itself.
            TickProfiler.StartIfAsked();
        }
    }
}
