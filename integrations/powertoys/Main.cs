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
    /// Runs `soos-cli --json` (see crates/soos-cli) on the query and shows the
    /// result. The calculator itself is soos-core; this only runs the binary
    /// and copies its answer.
    /// </summary>
    public class Main : IPlugin
    {
        public static string PluginID => "745B50E713684389B6424E3D2FD2AB3B";

        public string Name => "Soos";

        public string Description => "A calculator where every line is an expression: 20 inches in cm, $840 * 2, 6% off 40 EUR.";

        // soos-cli gives up on the network after 5 seconds.
        private static readonly TimeSpan CliTimeout = TimeSpan.FromSeconds(8);

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

            // One soos-cli.exe per keystroke.
            var exe = Environment.GetEnvironmentVariable("SOOS_EXE") ?? "soos-cli";
            var psi = new ProcessStartInfo
            {
                FileName = exe,
                RedirectStandardOutput = true,
                // soos-cli writes UTF-8 when piped; the default would decode
                // it with the console code page and mangle symbols like €.
                StandardOutputEncoding = System.Text.Encoding.UTF8,
                UseShellExecute = false,
                CreateNoWindow = true,
            };
            psi.ArgumentList.Add("--json");
            // So a query starting with `--` is an expression, not an option.
            psi.ArgumentList.Add("--");
            psi.ArgumentList.Add(search);

            string stdout;
            try
            {
                using var process = Process.Start(psi)!;
                var read = process.StandardOutput.ReadToEndAsync();
                if (!read.Wait(CliTimeout))
                {
                    try
                    {
                        process.Kill();
                    }
                    catch (InvalidOperationException)
                    {
                        // It exited in the meantime.
                    }
                    return Single("soos-cli didn't answer", $"No result after {CliTimeout.TotalSeconds:0} seconds.", string.Empty, false);
                }
                stdout = read.Result;
                process.WaitForExit();
            }
            catch (Win32Exception)
            {
                return Single(
                    "soos-cli.exe not found",
                    "Add it to PATH, or set the SOOS_EXE environment variable to its full path.",
                    string.Empty,
                    false);
            }

            // "result" is what the app shows, "value" what clicking it copies,
            // "detail" an error's full message.
            try
            {
                using var doc = JsonDocument.Parse(stdout);
                var root = doc.RootElement;
                if (root.GetProperty("ok").GetBoolean())
                {
                    var title = root.GetProperty("result").GetString() ?? string.Empty;
                    var copy = root.TryGetProperty("value", out var value)
                        ? value.GetString() ?? title
                        : title;
                    return Single(title, search, copy, true);
                }
                var error = root.GetProperty("error").GetString() ?? "error";
                var detail = root.TryGetProperty("detail", out var d) ? d.GetString() ?? search : search;
                return Single(error, detail, string.Empty, false);
            }
            catch (Exception e) when (e is JsonException or KeyNotFoundException or InvalidOperationException)
            {
                return Single("soos-cli gave an unexpected answer", "Check that SOOS_EXE points at soos-cli.exe.", string.Empty, false);
            }
        }

        private static List<Result> Single(string title, string subTitle, string copy, bool ok)
        {
            return new List<Result>
            {
                new Result
                {
                    Title = title,
                    SubTitle = subTitle,
                    IcoPath = "Images\\soos.dark.png",
                    Action = _ =>
                    {
                        if (ok)
                        {
                            Clipboard.SetText(copy);
                        }
                        return ok;
                    },
                },
            };
        }
    }
}
