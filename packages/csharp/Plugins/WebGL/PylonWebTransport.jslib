mergeInto(LibraryManager.library, {
  PylonWT_Available: function() { return typeof WebTransport !== 'undefined' ? 1 : 0; },
  PylonWT_Open__deps: ['$PylonWeb'],
  PylonWT_Open: function(url, hashes) {
    var h = { state: 0, queue: [], queued: 0, streams: [], datagrams: [], received: 0, pending: 0, datagramPending: 0, code: -1, reason: '' };
    var id = PylonWeb.add(h);
    var fail = function() { if (PylonWeb.handles[id]) PylonWeb.stop(h, -1, 'Browser WebTransport failed'); };
    try {
      var options = {}, pins = JSON.parse(UTF8ToString(hashes));
      if (pins.length) options.serverCertificateHashes = pins.map(function(pin) {
        var raw = atob(pin), bytes = new Uint8Array(raw.length);
        for (var i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
        return { algorithm: 'sha-256', value: bytes };
      });
      var session = h.session = new WebTransport(UTF8ToString(url), options);
      session.closed.then(function(info) {
        if (!PylonWeb.handles[id]) return;
        h.state = 2; h.code = info.closeCode; h.reason = info.reason || '';
      }).catch(fail);
      session.ready.then(function() {
        if (!PylonWeb.handles[id]) return;
        return session.createBidirectionalStream().then(function(stream) {
          if (!PylonWeb.handles[id]) { session.close(); return; }
          h.writer = stream.writable.getWriter();
          var writable = session.datagrams.createWritable ? session.datagrams.createWritable() : session.datagrams.writable;
          h.datagramWriter = writable.getWriter();
          h.streamReader = stream.readable.getReader();
          h.datagramReader = session.datagrams.readable.getReader();
          h.state = 1;
          var read = function(reader, queue) {
            reader.read().then(function(part) {
              if (!PylonWeb.handles[id] || h.state !== 1) return;
              if (part.done) { fail(); return; }
              if (h.received + part.value.length > 64 * 1024 * 1024 || queue.length >= 4096) { fail(); return; }
              queue.push(part.value); h.received += part.value.length;
              read(reader, queue);
            }).catch(fail);
          };
          read(h.streamReader, h.streams);
          read(h.datagramReader, h.datagrams);
        });
      }).catch(fail);
    } catch (_) { fail(); }
    return id;
  },
  PylonWT_MaxDatagram__deps: ['$PylonWeb'],
  PylonWT_MaxDatagram: function(id) {
    var h = PylonWeb.handles[id];
    return h && h.session ? (h.session.datagrams.maxDatagramSize || 1200) : 0;
  },
  PylonWT_Queued__deps: ['$PylonWeb'],
  PylonWT_Queued: function(id) { var h = PylonWeb.handles[id]; return h ? h.pending : 0; },
  PylonWT_Write__deps: ['$PylonWeb'],
  PylonWT_Write: function(id, ptr, length, datagram) {
    var h = PylonWeb.handles[id];
    if (!h || h.state !== 1) return 0;
    if (h.pending + h.datagramPending + length > 64 * 1024 * 1024) return 0;
    var field = datagram ? 'datagramPending' : 'pending';
    h[field] += length;
    try {
      (datagram ? h.datagramWriter : h.writer).write(HEAPU8.slice(ptr, ptr + length)).then(function() {
        h[field] -= length;
      }).catch(function() { if (PylonWeb.handles[id]) PylonWeb.stop(h, -1, 'Browser WebTransport write failed'); });
      return 1;
    } catch (_) { PylonWeb.stop(h, -1, 'Browser WebTransport write failed'); return 0; }
  },
  PylonWT_Read__deps: ['$PylonWeb'],
  PylonWT_Read: function(id, ptr, length, datagram) {
    var h = PylonWeb.handles[id];
    if (!h) return datagram ? -2 : -7;
    var queue = datagram ? h.datagrams : h.streams;
    if (!queue.length) return datagram ? -2 : (h.state === 2 ? -7 : 0);
    var bytes = queue[0];
    if (datagram && bytes.length > length) { queue.shift(); h.received -= bytes.length; return -3; }
    var n = Math.min(bytes.length, length);
    HEAPU8.set(bytes.subarray(0, n), ptr); h.received -= n;
    if (n === bytes.length) queue.shift(); else queue[0] = bytes.subarray(n);
    return n;
  }
});
