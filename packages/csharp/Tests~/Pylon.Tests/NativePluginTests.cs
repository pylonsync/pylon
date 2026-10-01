using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using Pylon.Realtime;
using Xunit;

namespace Pylon.Tests
{
    /// <summary>
    /// Runs when the WebTransport plugin for this host is next to the test
    /// assembly (packages/csharp/Plugins). With <c>PYLON_REQUIRE_PLUGIN</c>
    /// set, a missing plugin fails instead.
    /// </summary>
    sealed class PluginFactAttribute : FactAttribute
    {
        public static string FileName =>
            RuntimeInformation.IsOSPlatform(OSPlatform.Windows) ? "pylon_shard_client.dll"
            : RuntimeInformation.IsOSPlatform(OSPlatform.OSX) ? "libpylon_shard_client.dylib"
            : "libpylon_shard_client.so";

        public PluginFactAttribute()
        {
            if (!string.IsNullOrEmpty(Environment.GetEnvironmentVariable("PYLON_REQUIRE_PLUGIN"))) return;
            if (!File.Exists(Path.Combine(AppContext.BaseDirectory, FileName)))
                Skip = $"no {FileName} in packages/csharp/Plugins for this host";
        }
    }

    /// <summary>The C ABI from C#, without a server: loading, refused arguments, a failed session.</summary>
    public class NativePluginTests
    {
        [PluginFact]
        public void ThePluginLoadsWithTheAbiThisClientCalls()
        {
            var version = WebTransportNative.Version();
            Assert.NotNull(version);
            Assert.True(WebTransportNative.Compatible(version), version);
            Assert.True(NativeWebTransport.Instance.Available);
        }

        [PluginFact]
        public void RefusedArgumentsAndUnknownHandles()
        {
            Assert.Null(NativeWebTransport.Instance.Connect("http://127.0.0.1:1/shard", Array.Empty<byte[]>()));
            Assert.Null(NativeWebTransport.Instance.Connect("not a url", Array.Empty<byte[]>()));
            Assert.Equal(-1, WebTransportNative.pylon_wt_state(ulong.MaxValue));
            Assert.Equal(-1, WebTransportNative.SendDatagram(ulong.MaxValue, new byte[] { 1 }));
            Assert.Equal(-1L, WebTransportNative.StreamRead(ulong.MaxValue, new byte[8]));
            Assert.Equal(-1L, WebTransportNative.pylon_wt_stream_queued(ulong.MaxValue));
            WebTransportNative.pylon_wt_free(ulong.MaxValue);
        }

        [PluginFact]
        public void ASessionToAServerThatNeverAnswersFails()
        {
            // A bound socket that reads nothing: no handshake answer, and no
            // ICMP error, so the plugin's open timeout (10 s) ends the attempt.
            using var silent = new System.Net.Sockets.UdpClient(new System.Net.IPEndPoint(System.Net.IPAddress.Loopback, 0));
            var port = ((System.Net.IPEndPoint)silent.Client.LocalEndPoint!).Port;
            using var session = NativeWebTransport.Instance.Connect($"https://127.0.0.1:{port}/shard", new[] { new byte[32] })!;
            Assert.NotNull(session);
            var deadline = DateTime.UtcNow.AddSeconds(20);
            while (session.State == WebTransportNative.StateConnecting && DateTime.UtcNow < deadline) Thread.Sleep(20);
            Assert.Equal(WebTransportNative.StateFailed, session.State);
            Assert.Contains("no answer", session.Error);
            Assert.False(session.StreamWrite(new byte[] { 1 }));
            Assert.Equal(WebTransportNative.ErrNone, session.RecvDatagram(new byte[16]));
        }
    }
}
