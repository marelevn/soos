using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Text.Json;
using System.Windows;
using Wox.Plugin;

namespace Community.PowerToys.Run.Plugin.Soos
{
    /// <summary>
    /// Hands the query to `soos-cli --json` (see crates/soos-cli) and shows the
    /// result. All the calculator logic lives in soos-core; this is a thin
    /// shell that spawns the binary and copies its answer to the clipboard.
    /// </summary>
    public class Main : IPlugin
    {
        public static string PluginID => "745B50E713684389B6424E3D2FD2AB3B";

        public string Name => "Soos";

        public string Description => "A calculator where every line is an expression -- 20 inches in cm, $20 in euro - 5% discount.";

        private PluginInitContext? _context;

        public void Init(PluginInitContext context)
        {
            _context = context;
        }

        public List<Result> Query(Query query)
        {
            var search = query.Search?.Trim();
            if (string.IsNullOrEmpty(search))
            {
                return new List<Result>();
            }

            // Spawns a fresh soos-cli.exe (and reloads the rate cache from
            // disk) on every keystroke. A persistent `soos-cli --serve`
            // stdin loop would be the way to avoid that if this measurably lags.
            var exe = Environment.GetEnvironmentVariable("SOOS_EXE") ?? "soos-cli";
            var psi = new ProcessStartInfo
            {
                FileName = exe,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true,
            };
            psi.ArgumentList.Add("--json");
            psi.ArgumentList.Add(search);

            string stdout;
            try
            {
                using var process = Process.Start(psi);
                stdout = process!.StandardOutput.ReadToEnd();
                process.WaitForExit();
            }
            catch (Win32Exception)
            {
                return new List<Result>
                {
                    new Result
                    {
                        Title = "soos-cli.exe not found",
                        SubTitle = "Add it to PATH, or set the SOOS_EXE environment variable to its full path.",
                        IcoPath = "Images\\soos.dark.png",
                    },
                };
            }

            string title;
            bool ok;
            try
            {
                using var doc = JsonDocument.Parse(stdout);
                ok = doc.RootElement.GetProperty("ok").GetBoolean();
                title = ok
                    ? doc.RootElement.GetProperty("result").GetString() ?? string.Empty
                    : doc.RootElement.GetProperty("error").GetString() ?? "error";
            }
            catch (JsonException)
            {
                ok = false;
                title = "soos produced no output";
            }

            return new List<Result>
            {
                new Result
                {
                    Title = title,
                    SubTitle = search,
                    IcoPath = "Images\\soos.dark.png",
                    Action = _ =>
                    {
                        if (ok)
                        {
                            Clipboard.SetText(title);
                        }
                        return ok;
                    },
                },
            };
        }
    }
}
