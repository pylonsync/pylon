#nullable enable
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace Pylon.Realtime
{
    /// <summary>
    /// The WebTransport plugin (crates/shard-client-ffi, shipped under
    /// packages/csharp/Plugins). Unity's .NET has no QUIC, so a native client
    /// does the transport; this side polls it. See the crate's docs for the
    /// contract of each call.
    /// </summary>
    internal static class WebTransportNative
    {
#if (UNITY_IOS || UNITY_TVOS) && !UNITY_EDITOR
        // iOS links the plugin's framework into the app.
        const string Lib = "__Internal";
#else
        const string Lib = "pylon_shard_client";
#endif

        public const int StateConnecting = 0;
        public const int StateOpen = 1;
        public const int StateClosed = 2;
        public const int StateFailed = 3;

        public const int ErrNone = -2;
        public const int ErrBufferTooSmall = -3;
        public const int ErrStreamEnded = -7;

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern ulong pylon_wt_connect(byte[] url, byte[]? hashes, UIntPtr hashCount);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        public static extern int pylon_wt_state(ulong handle);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        public static extern int pylon_wt_max_datagram_size(ulong handle);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int pylon_wt_send_datagram(ulong handle, byte[] data, UIntPtr len);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int pylon_wt_recv_datagram(ulong handle, byte[] buf, UIntPtr cap);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int pylon_wt_stream_write(ulong handle, byte[] data, UIntPtr len);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern long pylon_wt_stream_read(ulong handle, byte[] buf, UIntPtr cap);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int pylon_wt_close(ulong handle, uint code, byte[]? reason, UIntPtr len);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int pylon_wt_close_info(ulong handle, out uint code, byte[] buf, UIntPtr cap);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern int pylon_wt_error(ulong handle, byte[] buf, UIntPtr cap);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        public static extern void pylon_wt_free(ulong handle);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        static extern IntPtr pylon_wt_version();

        /// <summary>The plugin ABI major version this client calls (crates/shard-client-ffi).</summary>
        public const int AbiMajor = 1;

        /// <summary>True when the plugin is loaded and has <see cref="AbiMajor"/>.</summary>
        public static bool Compatible(string? version) =>
            version != null && version.Split('.')[0] == AbiMajor.ToString(System.Globalization.CultureInfo.InvariantCulture);

        /// <summary>The plugin's version, or null when it is not loaded on this platform.</summary>
        public static string? Version()
        {
            try
            {
                var p = pylon_wt_version();
                return p == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(p);
            }
            catch (DllNotFoundException)
            {
                return null;
            }
            catch (EntryPointNotFoundException)
            {
                return null;
            }
        }

        /// <summary>Start a session. 0 when an argument is refused. Throws DllNotFoundException without the plugin.</summary>
        public static ulong Connect(string url, byte[][] hashes)
        {
            var urlBytes = Encoding.UTF8.GetBytes(url + "\0");
            byte[]? flat = null;
            if (hashes.Length > 0)
            {
                flat = new byte[hashes.Length * 32];
                for (var i = 0; i < hashes.Length; i++) Buffer.BlockCopy(hashes[i], 0, flat, i * 32, 32);
            }
            return pylon_wt_connect(urlBytes, flat, (UIntPtr)hashes.Length);
        }

        public static int SendDatagram(ulong handle, byte[] data) =>
            pylon_wt_send_datagram(handle, data, (UIntPtr)data.Length);

        public static int RecvDatagram(ulong handle, byte[] buf) => pylon_wt_recv_datagram(handle, buf, (UIntPtr)buf.Length);

        public static int StreamWrite(ulong handle, byte[] data) => pylon_wt_stream_write(handle, data, (UIntPtr)data.Length);

        public static long StreamRead(ulong handle, byte[] buf) => pylon_wt_stream_read(handle, buf, (UIntPtr)buf.Length);

        public static void Close(ulong handle, uint code, string reason)
        {
            var bytes = Encoding.UTF8.GetBytes(reason);
            pylon_wt_close(handle, code, bytes, (UIntPtr)bytes.Length);
        }

        /// <summary>The server's application close code and reason, when it closed the session with one.</summary>
        public static (uint Code, string Reason)? CloseInfo(ulong handle)
        {
            var buf = new byte[1024];
            var n = pylon_wt_close_info(handle, out var code, buf, (UIntPtr)buf.Length);
            if (n < 0) return null;
            return (code, Encoding.UTF8.GetString(buf, 0, Math.Min(n, buf.Length)));
        }

        public static string Error(ulong handle)
        {
            var buf = new byte[1024];
            var n = pylon_wt_error(handle, buf, (UIntPtr)buf.Length);
            return n <= 0 ? "" : Encoding.UTF8.GetString(buf, 0, Math.Min(n, buf.Length));
        }
    }

    /// <summary>One WebTransport session: the stream, datagrams, and how it ended. Polled; it calls nothing back.</summary>
    internal interface IWebTransportSession : IDisposable
    {
        /// <summary><see cref="WebTransportNative.StateConnecting"/>, open, closed, or failed.</summary>
        int State { get; }
        /// <summary>The largest datagram the session sends (0 when unknown).</summary>
        int MaxDatagramSize { get; }
        bool SendDatagram(byte[] data);
        /// <summary>The length of the next datagram copied into <paramref name="buf"/>, or a negative value when none waits.</summary>
        int RecvDatagram(byte[] buf);
        bool StreamWrite(byte[] data);
        /// <summary>Stream bytes copied into <paramref name="buf"/>: 0 when none wait, negative when the stream ended.</summary>
        long StreamRead(byte[] buf);
        void Close(uint code, string reason);
        /// <summary>The server's application close code and reason, when it closed the session with one.</summary>
        (uint Code, string Reason)? CloseInfo { get; }
        string Error { get; }
    }

    /// <summary>Opens WebTransport sessions. Tests replace it; the default is the native plugin.</summary>
    internal interface IWebTransportFactory
    {
        /// <summary>False when no plugin is loaded on this platform.</summary>
        bool Available { get; }
        /// <summary>Start a session, or null when the arguments are refused.</summary>
        IWebTransportSession? Connect(string url, byte[][] certHashes);
    }

    internal sealed class NativeWebTransport : IWebTransportFactory
    {
        public static readonly NativeWebTransport Instance = new NativeWebTransport();

        bool? _available;

        public bool Available => _available ??= WebTransportNative.Compatible(WebTransportNative.Version());

        public IWebTransportSession? Connect(string url, byte[][] certHashes)
        {
            var handle = WebTransportNative.Connect(url, certHashes);
            return handle == 0 ? null : new Session(handle);
        }

        sealed class Session : IWebTransportSession
        {
            readonly ulong _handle;
            int _freed;

            public Session(ulong handle)
            {
                _handle = handle;
            }

            public int State => WebTransportNative.pylon_wt_state(_handle);
            public int MaxDatagramSize => Math.Max(0, WebTransportNative.pylon_wt_max_datagram_size(_handle));
            public bool SendDatagram(byte[] data) => WebTransportNative.SendDatagram(_handle, data) == 0;
            public int RecvDatagram(byte[] buf) => WebTransportNative.RecvDatagram(_handle, buf);
            public bool StreamWrite(byte[] data) => WebTransportNative.StreamWrite(_handle, data) == 0;
            public long StreamRead(byte[] buf) => WebTransportNative.StreamRead(_handle, buf);
            public void Close(uint code, string reason) => WebTransportNative.Close(_handle, code, reason);
            public (uint Code, string Reason)? CloseInfo => WebTransportNative.CloseInfo(_handle);
            public string Error => WebTransportNative.Error(_handle);

            public void Dispose()
            {
                if (System.Threading.Interlocked.Exchange(ref _freed, 1) == 0) WebTransportNative.pylon_wt_free(_handle);
            }
        }
    }
}
