/*
 * Revtrail tracking snippet. Daily cookieless identity is the default. Install:
 *   <script defer src="https://your-revtrail/track.js" data-site="PUBLIC_ID"></script>
 *
 * Fires a "pageview" on load and on SPA navigations, and exposes a global
 * `revtrail(name, extra)` for custom conversion events, e.g.
 *   revtrail('signup')
 * Revenue is accepted only from a verified payment-provider webhook; a public
 * site key cannot authenticate financial data.
 *
 * Add data-identity="persistent" to opt into a random, site-scoped browser id
 * for multi-day retention. No cookies are used in either mode.
 */
(function () {
  var script = document.currentScript;
  if (!script) return;
  var site = script.getAttribute("data-site");
  if (!site) return;
  var persistentIdentity = script.getAttribute("data-identity") === "persistent";
  var identityKey = "revtrail_visitor:" + site;

  // Default to the script's origin. data-endpoint may be either a full relay
  // path (/api/fn/pv) or an alternate Revtrail origin.
  var configuredEndpoint = script.getAttribute("data-endpoint");
  var endpoint = configuredEndpoint;
  if (!configuredEndpoint) {
    try {
      endpoint = new URL(script.src).origin + "/api/fn/ingestEvent";
    } catch (e) {
      endpoint = "/api/fn/ingestEvent";
    }
  } else if (configuredEndpoint.indexOf("/api/fn/") === -1) {
    endpoint = configuredEndpoint.replace(/\/$/, "") + "/api/fn/ingestEvent";
  }

  // Site owners can exclude their own browser by opening a dashboard-provided
  // URL containing ?rt_opt_out=<site key>. This stores only the opt-out choice,
  // never an identifier, and scrubs the parameter before any event is sent.
  var optOutKey = "revtrail_opt_out:" + site;
  var optedOut = false;
  try {
    var privacyParams = new URLSearchParams(location.search);
    if (privacyParams.get("rt_opt_out") === site) {
      localStorage.setItem(optOutKey, "1");
      localStorage.removeItem(identityKey);
      privacyParams.delete("rt_opt_out");
      var privacyQuery = privacyParams.toString();
      history.replaceState(
        history.state,
        "",
        location.pathname + (privacyQuery ? "?" + privacyQuery : "") + location.hash
      );
    }
    optedOut = localStorage.getItem(optOutKey) === "1";
  } catch (e) {}

  if (optedOut) {
    var disabled = function () {};
    disabled.visitorId = function () { return null; };
    disabled.visitorIdAsync = function () { return Promise.resolve(null); };
    disabled.optOut = function () {};
    disabled.optIn = function () {
      try { localStorage.removeItem(optOutKey); } catch (e) {}
      location.reload();
    };
    window.revtrail = disabled;
    return;
  }

  function mintVisitorId() {
    try {
      if (crypto && crypto.randomUUID) return crypto.randomUUID();
      if (crypto && crypto.getRandomValues) {
        var bytes = new Uint8Array(16);
        crypto.getRandomValues(bytes);
        return Array.prototype.map.call(bytes, function (n) {
          return n.toString(16).padStart(2, "0");
        }).join("");
      }
    } catch (e) {}
    return Date.now().toString(36) + Math.random().toString(36).slice(2);
  }

  // Persistent identity is a deliberate site-level opt-in. Daily mode removes
  // a stale id if the owner switches back, preserving its cookieless contract.
  var persistentVisitorId = null;
  try {
    if (persistentIdentity) {
      persistentVisitorId = localStorage.getItem(identityKey);
      if (!persistentVisitorId) {
        persistentVisitorId = mintVisitorId();
        localStorage.setItem(identityKey, persistentVisitorId);
      }
    } else {
      localStorage.removeItem(identityKey);
    }
  } catch (e) {}

  function utm() {
    var q = new URLSearchParams(location.search);
    return {
      utmSource: q.get("utm_source") || undefined,
      utmMedium: q.get("utm_medium") || undefined,
      utmCampaign: q.get("utm_campaign") || undefined,
      utmContent: q.get("utm_content") || undefined,
    };
  }

  // The server-computed visitor hash, captured from the first beacon response.
  // Daily mode keeps it in memory only; persistent mode derives it from the
  // random site-scoped browser id above. Attach the hash to Stripe Checkout as
  // client_reference_id (or metadata.revtrail_visitor_id) via
  // revtrail.visitorId() and the payment webhook joins the visitor's journey.
  var vid = null;

  // Cross-domain / web-to-app continuity: a landing URL carrying ?rt_vid=…
  // (stamped by a sibling domain's decorated link, or a deep link from the
  // app) adopts that visitor id, so the journey continues across domains.
  // The param is scrubbed from the address bar immediately.
  var adopted = null;
  try {
    var landing = new URLSearchParams(location.search);
    adopted = landing.get("rt_vid");
    if (adopted) {
      vid = adopted;
      landing.delete("rt_vid");
      var qs = landing.toString();
      history.replaceState(
        history.state,
        "",
        location.pathname + (qs ? "?" + qs : "") + location.hash
      );
    }
  } catch (e) {}

  // Awaitable visitor id. visitorId() is sync and null until the first beacon
  // answers — a race for instant-checkout pages that read it on load. This
  // promise resolves with the id once it's known (immediately if rt_vid was
  // adopted), or null after a hard timeout so a blocked/failed beacon can
  // never hang checkout. Resolves exactly once.
  var readyResolve;
  var readyPromise = new Promise(function (res) { readyResolve = res; });
  function resolveReady() {
    if (readyResolve) { readyResolve(vid); readyResolve = null; }
  }
  if (vid !== null) resolveReady();
  setTimeout(resolveReady, 2500);

  function send(name, extra) {
    if (optedOut) return;
    var body = {
      site: site,
      name: name,
      path: location.pathname,
      hostname: location.hostname,
      referrer: document.referrer || undefined,
    };
    if (adopted) body.visitorHash = adopted;
    else if (persistentVisitorId) body.visitorId = persistentVisitorId;
    var u = utm();
    for (var k in u) if (u[k] !== undefined) body[k] = u[k];
    if (extra) for (var j in extra) body[j] = extra[j];

    var json = JSON.stringify(body);
    // First send uses fetch so we can read the visitor id back (sendBeacon has
    // no response); later sends prefer sendBeacon (survives page unload).
    if (vid === null || !navigator.sendBeacon) {
      fetch(endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: json,
        keepalive: true,
      })
        .then(function (r) { return r.json(); })
        .then(function (b) { if (b && b.visitorId) vid = b.visitorId; resolveReady(); })
        .catch(function () { resolveReady(); });
    } else {
      navigator.sendBeacon(endpoint, new Blob([json], { type: "application/json" }));
    }
  }

  // Pageview on load.
  send("pageview");

  // Pageviews on client-side (SPA) navigation.
  var push = history.pushState;
  history.pushState = function () {
    push.apply(this, arguments);
    send("pageview");
  };
  addEventListener("popstate", function () {
    send("pageview");
  });

  // Public API for custom conversion events.
  window.revtrail = function (name, extra) {
    send(name, extra);
  };
  // The visitor's server-computed id (null until the first beacon answers).
  window.revtrail.visitorId = function () {
    return vid;
  };
  // Awaitable form for instant-checkout paths: resolves the id once known, or
  // null after ~2.5s. Pass best-effort as Stripe client_reference_id; never
  // block or fail checkout on it.
  //   const vid = await revtrail.visitorIdAsync();
  window.revtrail.visitorIdAsync = function () {
    return readyPromise;
  };
  window.revtrail.optOut = function () {
    try {
      localStorage.setItem(optOutKey, "1");
      localStorage.removeItem(identityKey);
    } catch (e) {}
    optedOut = true;
    vid = null;
    persistentVisitorId = null;
  };
  window.revtrail.optIn = function () {
    try { localStorage.removeItem(optOutKey); } catch (e) {}
    optedOut = false;
  };

  // Outbound cross-domain continuity: with data-domains="app.example.com,
  // docs.example.com" on the snippet, clicks on links to those hosts get
  // ?rt_vid=<id> appended so the destination's snippet adopts the journey.
  var linkedDomains = (script.getAttribute("data-domains") || "")
    .split(",")
    .map(function (d) { return d.trim().toLowerCase(); })
    .filter(Boolean);
  if (linkedDomains.length) {
    addEventListener(
      "click",
      function (ev) {
        if (!vid) return;
        var a = ev.target && ev.target.closest && ev.target.closest("a[href]");
        if (!a) return;
        try {
          var u = new URL(a.href, location.href);
          var host = u.hostname.replace(/^www\./, "");
          if (host === location.hostname.replace(/^www\./, "")) return;
          if (linkedDomains.indexOf(host) === -1) return;
          u.searchParams.set("rt_vid", vid);
          a.href = u.toString();
        } catch (e) {}
      },
      true
    );
  }
})();
