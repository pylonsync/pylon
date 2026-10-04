#if UNITY_WEBGL && !UNITY_EDITOR
#nullable enable
using System;
using System.Runtime.InteropServices;
using System.Text;
using Pylon.Realtime;

namespace Pylon
{
    internal sealed class BrowserWebTransport : IWebTransportFactory
    {
        public static readonly BrowserWebTransport Instance = new BrowserWebTransport();
        [DllImport("__Internal")] static extern int PylonWT_Available();
        [DllImport("__Internal")] static extern int PylonWT_Open(string url, string hashes);
        [DllImport("__Internal")] static extern int PylonWT_MaxDatagram(int id);
        [DllImport("__Internal")] static extern int PylonWT_Write(int id, byte[] bytes, int length, int datagram);
        [DllImport("__Internal")] static extern int PylonWT_Read(int id, byte[] bytes, int length, int datagram);
        [DllImport("__Internal")] static extern int PylonWT_Queued(int id);
        public bool Available => PylonWT_Available() != 0;
        public IWebTransportSession? Connect(string url, byte[][] certHashes)
        {
            var hashes = new string[certHashes.Length];
            for (int i = 0; i < hashes.Length; i++) hashes[i] = Convert.ToBase64String(certHashes[i]);
            return new Session(PylonWT_Open(url, BrowserBridge.Json(hashes)));
        }
        sealed class Session : IWebTransportSession
        {
            int _id;
            public Session(int id) { _id = id; }
            public int State => BrowserBridge.PylonWeb_State(_id);
            public int MaxDatagramSize => PylonWT_MaxDatagram(_id);
            public bool SendDatagram(byte[] data) => PylonWT_Write(_id, data, data.Length, 1) == 1;
            public int RecvDatagram(byte[] buf) => PylonWT_Read(_id, buf, buf.Length, 1);
            public bool StreamWrite(byte[] data) => PylonWT_Write(_id, data, data.Length, 0) == 1;
            public long StreamQueued => PylonWT_Queued(_id);
            public long StreamRead(byte[] buf) => PylonWT_Read(_id, buf, buf.Length, 0);
            public void Close(uint code, string reason) => Dispose();
            public (uint Code, string Reason)? CloseInfo
            {
                get
                {
                    int code = BrowserBridge.PylonWeb_CloseCode(_id);
                    return code >= 0 ? ((uint)code, Error) : ((uint, string)?)null;
                }
            }
            public string Error
            {
                get
                {
                    var bytes = new byte[512];
                    var count = BrowserBridge.PylonWeb_CloseReason(_id, bytes, bytes.Length);
                    return Encoding.UTF8.GetString(bytes, 0, count);
                }
            }
            public void Dispose()
            {
                if (_id == 0) return;
                BrowserBridge.PylonWeb_Release(_id);
                _id = 0;
            }
        }
    }
}
#endif
