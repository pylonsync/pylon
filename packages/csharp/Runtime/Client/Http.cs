#nullable enable
using System;
using System.Collections.Generic;
using System.Net.Http;
using System.Threading;
using System.Threading.Tasks;

namespace Pylon
{
    /// <summary>One HTTP request the client sends.</summary>
    public sealed class PylonHttpRequest
    {
        public string Method { get; }
        public Uri Url { get; }
        public IReadOnlyDictionary<string, string> Headers { get; }
        /// <summary>The body, or null for none.</summary>
        public byte[]? Body { get; }

        public PylonHttpRequest(string method, Uri url, IReadOnlyDictionary<string, string> headers, byte[]? body)
        {
            Method = method;
            Url = url;
            Headers = headers;
            Body = body;
        }
    }

    /// <summary>The response to a <see cref="PylonHttpRequest"/>.</summary>
    public sealed class PylonHttpResponse
    {
        public int Status { get; }
        public byte[] Body { get; }

        public PylonHttpResponse(int status, byte[] body)
        {
            Status = status;
            Body = body ?? new byte[0];
        }
    }

    /// <summary>
    /// Sends HTTP requests for <see cref="PylonClient"/>. Replace it to test
    /// without a server or to route requests through your own stack. A
    /// failure to get a response throws <see cref="PylonException"/> with
    /// <see cref="PylonErrorKind.Transport"/>; any status is a response.
    /// </summary>
    public interface IPylonHttpTransport
    {
        Task<PylonHttpResponse> SendAsync(PylonHttpRequest request, CancellationToken cancellationToken);
    }

    /// <summary>The default transport, on <see cref="HttpClient"/>.</summary>
    public sealed class HttpClientTransport : IPylonHttpTransport, IDisposable
    {
        readonly HttpClient _http;
        readonly bool _owns;

        public HttpClientTransport(TimeSpan timeout)
        {
            _http = new HttpClient { Timeout = timeout };
            _owns = true;
        }

        /// <summary>Use your own <see cref="HttpClient"/>. It is not disposed with this transport.</summary>
        public HttpClientTransport(HttpClient http)
        {
            _http = http ?? throw new ArgumentNullException(nameof(http));
        }

        public async Task<PylonHttpResponse> SendAsync(PylonHttpRequest request, CancellationToken cancellationToken)
        {
            using var message = new HttpRequestMessage(new HttpMethod(request.Method), request.Url);
            string? contentType = null;
            foreach (var h in request.Headers)
            {
                if (string.Equals(h.Key, "Content-Type", StringComparison.OrdinalIgnoreCase))
                {
                    contentType = h.Value;
                    continue;
                }
                message.Headers.TryAddWithoutValidation(h.Key, h.Value);
            }
            if (request.Body != null)
            {
                message.Content = new ByteArrayContent(request.Body);
                if (contentType != null) message.Content.Headers.TryAddWithoutValidation("Content-Type", contentType);
            }
            try
            {
                using var response = await _http.SendAsync(message, cancellationToken).ConfigureAwait(false);
                var body = await response.Content.ReadAsByteArrayAsync().ConfigureAwait(false);
                return new PylonHttpResponse((int)response.StatusCode, body);
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                throw;
            }
            catch (OperationCanceledException e)
            {
                throw PylonException.Transport($"{request.Method} {request.Url.AbsolutePath} timed out", e);
            }
            catch (HttpRequestException e)
            {
                throw PylonException.Transport($"{request.Method} {request.Url.AbsolutePath} failed: {e.Message}", e);
            }
        }

        public void Dispose()
        {
            if (_owns) _http.Dispose();
        }
    }
}
