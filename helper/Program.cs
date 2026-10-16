// lintel-hw: the one process of METEOR V1 that runs as administrator
// (docs/ARCHITECTURE.md §11). It reads temperatures, fans, processor power and
// drive health through LibreHardwareMonitorLib, sets fan speeds inside
// FanPolicy, and talks to exactly one process: the one that launched it.
//
//   lintel-hw --parent-pid <pid> --parent-path <lintel.exe> --pipe <random>
//
// It serves \\.\pipe\lintel-hw-<userSid>-<random>, refuses every client that is
// not that parent (by image path and by process), hands the fans back to the
// motherboard on any exit, when the parent goes or when its heartbeat stops,
// and exits by itself after 30 s with nobody connected.
//
// It has no other mode. In particular it does not install itself, a service
// or a scheduled task: "start automatically" is not built (see the note at
// the end of this file).

using System.Diagnostics;
using System.IO.Pipes;
using System.Runtime.InteropServices;

// Our own Win32 calls come from System32 and nowhere else.
[assembly: DefaultDllImportSearchPaths(DllImportSearchPath.System32)]

namespace NotchHw;

internal sealed record Options(uint ParentPid, string ParentPath, string Token, string? Fake)
{
    /// Null for anything that is not exactly the form above.
    public static Options? Parse(string[] args)
    {
        uint pid = 0;
        string? path = null, token = null, fake = null;
        for (var i = 0; i + 1 < args.Length; i += 2)
        {
            var value = args[i + 1];
            switch (args[i])
            {
                case "--parent-pid" when uint.TryParse(value, out pid) && pid != 0:
                    break;
                case "--parent-path":
                    path = value;
                    break;
                case "--pipe":
                    token = value;
                    break;
#if DEBUG
                case "--fake":
                    fake = value;
                    break;
#endif
                default:
                    return null;
            }
        }
        if (args.Length % 2 != 0 || pid == 0 || path == null || !Path.IsPathFullyQualified(path)) return null;
        // The name of a pipe is not a place for anything but letters and digits.
        if (token == null || token.Length is < 16 or > 64 || !token.All(char.IsAsciiLetterOrDigit)) return null;
        return new Options(pid, Path.GetFullPath(path), token, fake);
    }
}

internal static class Program
{
    [Conditional("DEBUG")]
    public static void Log(string line) => Console.Error.WriteLine(line);

    private static int Main(string[] args)
    {
        var options = Options.Parse(args);
        if (options == null) return 2;
        try
        {
            return Run(options).GetAwaiter().GetResult();
        }
        catch (Exception e)
        {
            Log($"failed: {e}");
            return 1;
        }
    }

    private static bool SamePath(string? a, string b) => a != null && string.Equals(Path.GetFullPath(a), b, StringComparison.OrdinalIgnoreCase);

    private static async Task<int> Run(Options options)
    {
        // Who launched us. Its process stays open for as long as we run, so its
        // PID cannot become somebody else's; its account is who may use the pipe.
        var parent = Native.OpenProcess(options.ParentPid);
        if (parent == IntPtr.Zero || Native.Exited(parent) || !SamePath(Native.ImagePath(parent), options.ParentPath)) return 3;
        if (Native.UserSid(parent) is not { } sid) return 3;

#if DEBUG
        ISensorSource source = options.Fake != null ? new FakeSource(options.Fake) : new LhmSource();
#else
        ISensorSource source = new LhmSource();
#endif
        var engine = new Engine(source, options.Fake != null);
        // However the process ends, short of being killed: the fans go back first.
        // debt: killed outright, a motherboard keeps the last speed it was given until the next restart.
        AppDomain.CurrentDomain.ProcessExit += (_, _) => engine.Shutdown();
        AppDomain.CurrentDomain.UnhandledException += (_, _) => engine.Shutdown();
        Native.OnResume(() => engine.Resumed = true);

        using var stop = new CancellationTokenSource();
        try
        {
            using var pipe = Native.CreatePipe($@"\\.\pipe\lintel-hw-{sid}-{options.Token}", sid);
            Log("listening");

            _ = Task.Run(async () =>
            {
                using var timer = new PeriodicTimer(TimeSpan.FromSeconds(1));
                try
                {
                    while (await timer.WaitForNextTickAsync(stop.Token))
                    {
                        if (Native.Exited(parent))
                        {
                            Log("exit: the parent is gone");
                            break;
                        }
                        engine.Tick();
                        if (engine.Idle)
                        {
                            Log("exit: idle");
                            break;
                        }
                    }
                }
                catch (OperationCanceledException) { }
                catch (Exception e)
                {
                    Log($"exit: {e}");
                }
                stop.Cancel();
            });

            while (!stop.IsCancellationRequested && !engine.Quit)
            {
                await pipe.WaitForConnectionAsync(stop.Token);

                // The peer check, on every connection, before one byte is read:
                // the client runs the parent's file, and is the parent itself.
                var pid = Native.ClientPid(pipe);
                var client = pid is { } id ? Native.OpenProcess(id) : IntPtr.Zero;
                var refused = client == IntPtr.Zero ? "no client process"
                    : !SamePath(Native.ImagePath(client), options.ParentPath) ? "image path"
                    : pid != options.ParentPid ? "not the parent process"
                    : null;
                if (client != IntPtr.Zero) Native.CloseHandle(client);
                if (refused != null)
                {
                    Log($"refused: {refused}");
                    pipe.Disconnect();
                    continue;
                }

                using var connection = CancellationTokenSource.CreateLinkedTokenSource(stop.Token);
                using var writing = new SemaphoreSlim(1, 1);
                // One line at a time, replies and readings alike; a client that
                // stops reading is dropped rather than waited for.
                async Task<bool> Send(byte[] line)
                {
                    try
                    {
                        await writing.WaitAsync(connection.Token);
                        try
                        {
                            using var limit = CancellationTokenSource.CreateLinkedTokenSource(connection.Token);
                            limit.CancelAfter(5000);
                            await pipe.WriteAsync(line, limit.Token);
                            return true;
                        }
                        finally
                        {
                            writing.Release();
                        }
                    }
                    catch (Exception e) when (e is IOException or OperationCanceledException or ObjectDisposedException or InvalidOperationException)
                    {
                        try { connection.Cancel(); } catch (ObjectDisposedException) { }
                        return false;
                    }
                }

                Log("connected");
                engine.Connected(line => _ = Send(line));
                try
                {
                    await Serve(pipe, engine, Send, connection.Token);
                }
                catch (Exception e) when (e is IOException or OperationCanceledException) { }
                engine.Connected(null);
                connection.Cancel();
                // A reading still on its way out finishes or gives up before the next client.
                await writing.WaitAsync(CancellationToken.None);
                Log("disconnected");
                try { pipe.Disconnect(); } catch (Exception e) when (e is IOException or InvalidOperationException) { }
            }
        }
        catch (OperationCanceledException) { }
        finally
        {
            stop.Cancel();
            engine.Shutdown();
            Log("fans: automatic");
        }
        return 0;
    }

    /// Reads requests until the client goes, asks to quit, or sends a line too long.
    private static async Task Serve(NamedPipeServerStream pipe, Engine engine, Func<byte[], Task<bool>> send, CancellationToken cancel)
    {
        var buffer = new byte[Engine.MaxLine + 1];
        var have = 0;
        while (true)
        {
            var n = await pipe.ReadAsync(buffer.AsMemory(have), cancel);
            if (n == 0) return;
            have += n;
            var start = 0;
            while (Array.IndexOf(buffer, (byte)'\n', start, have - start) is >= 0 and var end)
            {
                if (!await send(engine.Handle(buffer.AsSpan(start, end - start))) || engine.Quit) return;
                start = end + 1;
            }
            have -= start;
            Buffer.BlockCopy(buffer, start, buffer, 0, have);
            if (have > Engine.MaxLine)
            {
                // No line of ours is this long: said once, then the connection ends.
                Log("refused: too large");
                await send(Engine.Error(null, null, "too large"));
                return;
            }
        }
    }
}

// "Start automatically" (docs/ARCHITECTURE.md §11: one Task Scheduler task,
// RunLevel=Highest) is deliberately not here. Such a task starts this program
// as administrator with no prompt, for anything that runs as the user. Two
// things then make it a way to become administrator unasked: the app is
// installed in the user's own folder, where this file can be replaced; and the
// .NET runtime loads a profiler named by environment variables
// (CORECLR_PROFILER_PATH), which the user's own registry sets for the task.
// It needs a copy under Program Files and a native launcher that starts it
// with a clean environment, proved with the owner at the machine.
