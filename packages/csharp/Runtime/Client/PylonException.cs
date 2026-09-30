#nullable enable
using System;
using System.Globalization;
using System.Text;

namespace Pylon
{
    /// <summary>What kind of failure a <see cref="PylonException"/> is.</summary>
    public enum PylonErrorKind
    {
        /// <summary>The server answered with a status other than 2xx.</summary>
        Http,
        /// <summary>The request did not complete: DNS, TLS, a timeout, a dropped connection.</summary>
        Transport,
        /// <summary>A body or frame did not have the expected shape.</summary>
        Decoding,
        /// <summary>The caller passed something invalid.</summary>
        InvalidArgument,
    }

    /// <summary>
    /// The one exception type the SDK throws. For an HTTP error,
    /// <see cref="Status"/> is the status and <see cref="Code"/> and
    /// <see cref="ServerMessage"/> come from the server's
    /// <c>{"error":{"code","message"}}</c> body (for example
    /// <c>UNAUTHENTICATED</c>, <c>RATE_LIMITED</c>).
    /// </summary>
    public sealed class PylonException : Exception
    {
        public PylonErrorKind Kind { get; }

        /// <summary>The HTTP status for <see cref="PylonErrorKind.Http"/>; otherwise 0.</summary>
        public int Status { get; }

        /// <summary>The server's error code, when the body had one.</summary>
        public string? Code { get; }

        /// <summary>The server's error message, when the body had one.</summary>
        public string? ServerMessage { get; }

        PylonException(PylonErrorKind kind, int status, string? code, string? serverMessage, string message, Exception? inner)
            : base(message, inner)
        {
            Kind = kind;
            Status = status;
            Code = code;
            ServerMessage = serverMessage;
        }

        public static PylonException Http(int status, string? code, string? message)
        {
            var sb = new StringBuilder("HTTP ").Append(status.ToString(CultureInfo.InvariantCulture));
            if (code != null) sb.Append(' ').Append(code);
            if (!string.IsNullOrEmpty(message)) sb.Append(": ").Append(message);
            return new PylonException(PylonErrorKind.Http, status, code, message, sb.ToString(), null);
        }

        public static PylonException Transport(string message, Exception? inner = null) =>
            new PylonException(PylonErrorKind.Transport, 0, null, null, message, inner);

        public static PylonException Decoding(string message, Exception? inner = null) =>
            new PylonException(PylonErrorKind.Decoding, 0, null, null, message, inner);

        public static PylonException InvalidArgument(string message) =>
            new PylonException(PylonErrorKind.InvalidArgument, 0, null, null, message, null);

        /// <summary>
        /// The error for a non-2xx response. Reads the wrapped
        /// <c>{"error":{"code","message"}}</c> body first, then a flat
        /// <c>{"code","message"}</c>, then keeps the raw text as the message.
        /// </summary>
        public static PylonException FromResponse(int status, byte[]? body)
        {
            var text = body == null || body.Length == 0 ? null : Encoding.UTF8.GetString(body);
            if (text != null)
            {
                PylonValue? parsed = null;
                try
                {
                    parsed = PylonValue.Parse(text);
                }
                catch (PylonException)
                {
                }
                if (parsed != null && parsed.Kind == PylonValueKind.Object)
                {
                    var inner = parsed["error"];
                    var code = inner["code"].AsStringOr(null);
                    var message = inner["message"].AsStringOr(null);
                    if (code != null || message != null) return Http(status, code, message);
                    code = parsed["code"].AsStringOr(null);
                    message = parsed["message"].AsStringOr(null);
                    if (code != null || message != null) return Http(status, code, message);
                }
            }
            return Http(status, null, text);
        }
    }
}
