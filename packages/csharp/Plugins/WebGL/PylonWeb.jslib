mergeInto(LibraryManager.library, {
  $PylonWeb: {
    next: 1,
    handles: {},
    add: function(value) { var id = PylonWeb.next++; PylonWeb.handles[id] = value; return id; },
    bytes: function(text) { return new TextEncoder().encode(text); },
    stop: function(h, code, reason) {
      h.state = 2; h.code = code; h.reason = reason;
      h.queue = []; h.queued = 0;
      if (h.session) {
        try { h.session.close(); } catch (_) {}
        if (h.streamReader) h.streamReader.cancel().catch(function() {});
        if (h.datagramReader) h.datagramReader.cancel().catch(function() {});
      }
      if (h.socket) {
        h.socket.onopen = h.socket.onmessage = h.socket.onerror = h.socket.onclose = null;
        try { h.socket.close(1000); } catch (_) {}
      }
    }
  },
  PylonWeb_Open__deps: ['$PylonWeb'],
  PylonWeb_Open: function(url, protocols, maxBytes) {
    var h = { state: 0, queue: [], queued: 0, max: maxBytes, code: 1006, reason: '' };
    var id = PylonWeb.add(h);
    try {
      var ws = h.socket = new WebSocket(UTF8ToString(url), JSON.parse(UTF8ToString(protocols)));
      ws.binaryType = 'arraybuffer';
      ws.onopen = function() { h.state = 1; };
      ws.onmessage = function(e) {
        var text = typeof e.data === 'string';
        var bytes = text ? PylonWeb.bytes(e.data) : new Uint8Array(e.data);
        if (h.queued + bytes.length > h.max || h.queue.length >= 4096) {
          PylonWeb.stop(h, 1009, 'Browser receive queue limit exceeded');
          return;
        }
        h.queue.push({ bytes: bytes, text: text }); h.queued += bytes.length;
      };
      ws.onerror = function() { PylonWeb.stop(h, 1006, 'Browser WebSocket failed'); };
      ws.onclose = function(e) { h.state = 2; h.code = e.code; h.reason = e.reason; };
    } catch (_) { PylonWeb.stop(h, 1006, 'Browser WebSocket could not open'); }
    return id;
  },
  PylonWeb_State__deps: ['$PylonWeb'],
  PylonWeb_State: function(id) { var h = PylonWeb.handles[id]; return h ? h.state : 2; },
  PylonWeb_Next__deps: ['$PylonWeb'],
  PylonWeb_Next: function(id) { var h = PylonWeb.handles[id]; return h && h.queue.length ? h.queue[0].bytes.length : -1; },
  PylonWeb_Read__deps: ['$PylonWeb'],
  PylonWeb_Read: function(id, ptr) {
    var h = PylonWeb.handles[id], m = h.queue.shift();
    HEAPU8.set(m.bytes, ptr); h.queued -= m.bytes.length;
    return m.text ? 1 : 0;
  },
  PylonWeb_Send__deps: ['$PylonWeb'],
  PylonWeb_Send: function(id, ptr, length, text) {
    var h = PylonWeb.handles[id];
    if (!h || h.state !== 1 || length > h.max) return -1;
    if (h.socket.bufferedAmount + length > h.max) return 0;
    try {
      var bytes = HEAPU8.slice(ptr, ptr + length);
      h.socket.send(text ? new TextDecoder().decode(bytes) : bytes);
      return 1;
    } catch (_) { PylonWeb.stop(h, 1006, 'Browser WebSocket send failed'); return -1; }
  },
  PylonWeb_CloseCode__deps: ['$PylonWeb'],
  PylonWeb_CloseCode: function(id) { var h = PylonWeb.handles[id]; return h ? h.code : 1006; },
  PylonWeb_CloseReason__deps: ['$PylonWeb'],
  PylonWeb_CloseReason: function(id, ptr, length) {
    var h = PylonWeb.handles[id], bytes = PylonWeb.bytes(h ? h.reason : 'Closed').subarray(0, length);
    HEAPU8.set(bytes, ptr); return bytes.length;
  },
  PylonWeb_Release__deps: ['$PylonWeb'],
  PylonWeb_Release: function(id) {
    var h = PylonWeb.handles[id];
    if (!h) return;
    delete PylonWeb.handles[id];
    PylonWeb.stop(h, 1000, 'Disposed');
    if (h.controller) h.controller.abort();
  },
  PylonWeb_Fetch__deps: ['$PylonWeb'],
  PylonWeb_Fetch: function(url, method, headers, ptr, length) {
    var h = { state: 0, status: 0, queue: [], queued: 0, controller: new AbortController() };
    var id = PylonWeb.add(h);
    try {
      fetch(UTF8ToString(url), {
        method: UTF8ToString(method), headers: JSON.parse(UTF8ToString(headers)),
        body: length < 0 ? undefined : HEAPU8.slice(ptr, ptr + length),
        credentials: 'omit', signal: h.controller.signal
      }).then(function(r) {
        return r.arrayBuffer().then(function(body) {
          if (!PylonWeb.handles[id]) return;
          var bytes = new Uint8Array(body);
          h.status = r.status; h.queue.push({ bytes: bytes, text: false });
          h.queued = bytes.length; h.state = 1;
        });
      }).catch(function() { if (PylonWeb.handles[id]) h.state = 2; });
    } catch (_) { h.state = 2; }
    return id;
  },
  PylonWeb_Status__deps: ['$PylonWeb'],
  PylonWeb_Status: function(id) { var h = PylonWeb.handles[id]; return h ? h.status : 0; }
});
