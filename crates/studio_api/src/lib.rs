use pylon_kernel::AppManifest;

/// Generate the Studio inspector as a full React + Tailwind app.
/// Uses CDN imports so no build step needed.
pub fn generate_studio_html(manifest: &AppManifest, api_base: &str) -> String {
    let manifest_json = serde_json::to_string(manifest).unwrap_or_else(|_| "{}".into());
    let _manifest_pretty = serde_json::to_string_pretty(manifest).unwrap_or_else(|_| "{}".into());

    let entity_names: Vec<&str> = manifest.entities.iter().map(|e| e.name.as_str()).collect();
    let entity_json = serde_json::to_string(&entity_names).unwrap_or_else(|_| "[]".into());

    let _query_names: Vec<&str> = manifest.queries.iter().map(|q| q.name.as_str()).collect();
    let _action_names: Vec<&str> = manifest.actions.iter().map(|a| a.name.as_str()).collect();
    let _policy_names: Vec<&str> = manifest.policies.iter().map(|p| p.name.as_str()).collect();
    let _route_paths: Vec<&str> = manifest.routes.iter().map(|r| r.path.as_str()).collect();

    // XSS prevention. The manifest is developer-authored but can contain
    // user-shaped strings (entity names, route paths, app name from
    // package.json), so treat every interpolated value as untrusted:
    //
    //   - JSON embedded inside <script>: escape `</script` so a crafted
    //     entity name can't close the script tag early.
    //   - Raw strings embedded in HTML (title, JSX text): HTML-encode all
    //     metacharacters so nothing interprets as markup.
    //   - Raw strings embedded as JS string literals (api_base): escape
    //     backslashes and quotes.
    //
    // Without these, `manifest.name = "</title><script>alert(1)</script>"`
    // was a one-line XSS on the Studio page.
    /// Escape `</script` (case-insensitive) and the JS line separators
    /// U+2028 / U+2029 inside JSON embedded in a `<script>` block.
    ///
    /// Previously this only replaced three exact casings (`script`/
    /// `Script`/`SCRIPT`). HTML tag matching is ASCII-case-insensitive,
    /// so `</ScRiPt>` still closed the script tag — a one-shot XSS via a
    /// crafted entity name. Now: any `</script` regardless of case gets
    /// a backslash inserted between the `<` and the `/`, which is still
    /// valid JSON (backslashes can escape any char) and no longer looks
    /// like a closing tag to the HTML parser.
    ///
    /// U+2028 and U+2029 are JS "line terminators" that close unclosed
    /// string literals and break Babel's parser; escape them as \u00XX-
    /// style sequences inside the JSON.
    fn escape_script_json(s: &str) -> String {
        // Compare on an ASCII-lowercased view (keeps byte indices aligned
        // with the original) so the match is case-insensitive, then write
        // from the original preserving non-ASCII bytes verbatim.
        let lower = s.to_ascii_lowercase();
        let sb = s.as_bytes();
        let lb = lower.as_bytes();
        let needle = b"</script";
        let mut out = String::with_capacity(s.len() + 8);
        let mut i = 0;
        while i < sb.len() {
            if i + needle.len() <= sb.len() && &lb[i..i + needle.len()] == needle {
                // Insert a backslash between `<` and `/` so the HTML
                // parser no longer sees a closing tag, but the JSON
                // remains valid (backslash-escape of `/` is allowed).
                out.push('<');
                out.push('\\');
                out.push_str(&s[i + 1..i + needle.len()]);
                i += needle.len();
            } else {
                // Append one char (not one byte) to avoid corrupting
                // multi-byte UTF-8 sequences like U+2028.
                let c = s[i..].chars().next().expect("mid-string must yield a char");
                out.push(c);
                i += c.len_utf8();
            }
        }
        // U+2028 / U+2029 are JS line terminators — close unclosed string
        // literals in Babel's parser. Escape inside the JSON.
        out.replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
    }
    fn html_escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&#39;"),
                _ => out.push(c),
            }
        }
        out
    }
    fn js_string_escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\u{2028}' => out.push_str("\\u2028"),
                '\u{2029}' => out.push_str("\\u2029"),
                '<' => out.push_str("\\u003c"), // also covers </script in JS context
                _ => out.push(c),
            }
        }
        out
    }
    let manifest_json = escape_script_json(&manifest_json);
    let entity_json = escape_script_json(&entity_json);
    let name_safe = html_escape(&manifest.name);
    let version_safe = html_escape(&manifest.version);
    let api_base_safe = js_string_escape(api_base);

    format!(
        r##"<!DOCTYPE html>
<html lang="en" class="dark">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{name} — pylon Studio</title>
  <script src="https://cdn.tailwindcss.com"></script>
  <!-- React 19 dropped UMD builds — the v19 URLs return 404 and Studio
       fails to boot with "ReactDOM is not defined". Pin to React 18
       which still ships UMD. Upgrade path when we move Studio to a
       bundled build: drop these scripts entirely. -->
  <script crossorigin src="https://unpkg.com/react@18/umd/react.production.min.js"></script>
  <script crossorigin src="https://unpkg.com/react-dom@18/umd/react-dom.production.min.js"></script>
  <script src="https://unpkg.com/@babel/standalone/babel.min.js"></script>
  <script>
    tailwind.config = {{
      darkMode: 'class',
      theme: {{
        extend: {{
          colors: {{
            border: '#27272a',
            input: '#27272a',
            ring: '#a1a1aa',
            background: '#09090b',
            foreground: '#fafafa',
            primary: {{ DEFAULT: '#fafafa', foreground: '#18181b' }},
            secondary: {{ DEFAULT: '#27272a', foreground: '#fafafa' }},
            destructive: {{ DEFAULT: '#ef4444', foreground: '#fafafa' }},
            muted: {{ DEFAULT: '#27272a', foreground: '#a1a1aa' }},
            accent: {{ DEFAULT: '#27272a', foreground: '#fafafa' }},
            card: {{ DEFAULT: '#09090b', foreground: '#fafafa' }},
          }},
          borderRadius: {{ lg: '0.5rem', md: 'calc(0.5rem - 2px)', sm: 'calc(0.5rem - 4px)' }},
        }}
      }}
    }}
  </script>
  <style>
    body {{ background: #09090b; color: #fafafa; font-family: system-ui, -apple-system, sans-serif; }}
    .card {{ background: #18181b; border: 1px solid #27272a; border-radius: 0.5rem; }}
    .btn {{ padding: 0.5rem 1rem; border-radius: 0.375rem; font-size: 0.875rem; font-weight: 500; cursor: pointer; transition: all 0.15s; }}
    .btn-primary {{ background: #fafafa; color: #18181b; }}
    .btn-primary:hover {{ background: #e4e4e7; }}
    .btn-secondary {{ background: #27272a; color: #fafafa; border: 1px solid #3f3f46; }}
    .btn-secondary:hover {{ background: #3f3f46; }}
    .btn-destructive {{ background: #ef4444; color: white; }}
    .btn-destructive:hover {{ background: #dc2626; }}
    .btn-ghost {{ background: transparent; color: #a1a1aa; }}
    .btn-ghost:hover {{ background: #27272a; color: #fafafa; }}
    .input {{ background: #09090b; border: 1px solid #27272a; border-radius: 0.375rem; padding: 0.5rem 0.75rem; color: #fafafa; font-size: 0.875rem; width: 100%; outline: none; }}
    .input:focus {{ border-color: #a1a1aa; box-shadow: 0 0 0 2px rgba(161,161,170,0.2); }}
    .badge {{ display: inline-flex; align-items: center; padding: 0.125rem 0.625rem; border-radius: 9999px; font-size: 0.75rem; font-weight: 500; }}
    .badge-secondary {{ background: #27272a; color: #a1a1aa; }}
    .badge-green {{ background: #064e3b; color: #6ee7b7; }}
    .badge-yellow {{ background: #78350f; color: #fcd34d; }}
    .badge-red {{ background: #7f1d1d; color: #fca5a5; }}
    .badge-blue {{ background: #1e3a5f; color: #93c5fd; }}
    .badge-purple {{ background: #3b0764; color: #d8b4fe; }}
    table {{ width: 100%; border-collapse: collapse; font-size: 0.8125rem; }}
    th {{ text-align: left; padding: 0.5rem 0.75rem; color: #a1a1aa; font-weight: 500; border-bottom: 1px solid #27272a; }}
    td {{ padding: 0.5rem 0.75rem; border-bottom: 1px solid #27272a; }}
    tr:hover {{ background: #18181b; }}
    .tab {{ padding: 0.5rem 1rem; cursor: pointer; border-bottom: 2px solid transparent; color: #a1a1aa; font-size: 0.875rem; }}
    .tab.active {{ color: #fafafa; border-color: #fafafa; }}
    .tab:hover {{ color: #fafafa; }}
    pre {{ background: #18181b; padding: 1rem; border-radius: 0.5rem; overflow-x: auto; font-size: 0.8125rem; border: 1px solid #27272a; }}
    code {{ font-family: 'SF Mono', 'Cascadia Code', 'Fira Code', monospace; }}
  </style>
</head>
<body>
  <div id="root"></div>
  <script type="text/babel">
    const API = "{api_base}";
    const MANIFEST = {manifest_json};
    const ENTITIES = {entity_json};

    function formatUptime(seconds) {{
      const h = Math.floor(seconds / 3600);
      const m = Math.floor((seconds % 3600) / 60);
      const s = Math.floor(seconds % 60);
      return `${{h}}h ${{m}}m ${{s}}s`;
    }}

    function kindBadgeClass(kind) {{
      switch (kind) {{
        case "insert": return "badge badge-green";
        case "update": return "badge badge-yellow";
        case "delete": return "badge badge-red";
        default: return "badge badge-secondary";
      }}
    }}

    function App() {{
      const [tab, setTab] = React.useState("entities");
      const [entity, setEntity] = React.useState(ENTITIES[0] || "");
      const [rows, setRows] = React.useState([]);
      const [loading, setLoading] = React.useState(false);
      const [showInsert, setShowInsert] = React.useState(false);
      const [insertJson, setInsertJson] = React.useState("{{}}");
      const [error, setError] = React.useState(null);
      const [syncLog, setSyncLog] = React.useState([]);

      // Users tab state
      const [users, setUsers] = React.useState([]);
      const [usersLoading, setUsersLoading] = React.useState(false);
      const [usersSearch, setUsersSearch] = React.useState("");
      const [sessionMsg, setSessionMsg] = React.useState(null);

      // Audit tab state
      const [auditLog, setAuditLog] = React.useState([]);
      const [auditEntityFilter, setAuditEntityFilter] = React.useState("");

      // Rooms tab state
      const [rooms, setRooms] = React.useState([]);
      const [roomsLoading, setRoomsLoading] = React.useState(false);
      const [expandedRoom, setExpandedRoom] = React.useState(null);
      const [roomMembers, setRoomMembers] = React.useState([]);
      const [roomMembersLoading, setRoomMembersLoading] = React.useState(false);

      // Health tab state
      const [health, setHealth] = React.useState(null);
      const [metrics, setMetrics] = React.useState(null);
      const [healthLoading, setHealthLoading] = React.useState(false);

      // Functions tab state
      const [fns, setFns] = React.useState([]);
      const [fnsLoading, setFnsLoading] = React.useState(false);
      const [fnTraces, setFnTraces] = React.useState([]);
      const [selectedTrace, setSelectedTrace] = React.useState(null);
      const [fnInvokeName, setFnInvokeName] = React.useState("");
      const [fnInvokeArgs, setFnInvokeArgs] = React.useState("{{}}");
      const [fnInvokeResult, setFnInvokeResult] = React.useState(null);

      const loadFns = async () => {{
        setFnsLoading(true);
        try {{
          const [fnsRes, tracesRes] = await Promise.all([
            fetch(`${{API}}/api/fn`),
            fetch(`${{API}}/api/fn/traces`),
          ]);
          const fnsData = await fnsRes.json();
          const tracesData = await tracesRes.json();
          setFns(Array.isArray(fnsData) ? fnsData : (fnsData.data || []));
          setFnTraces(Array.isArray(tracesData) ? tracesData : (tracesData.data || []));
        }} catch (err) {{
          setError(err.message);
        }}
        setFnsLoading(false);
      }};

      const invokeFn = async () => {{
        try {{
          const args = fnInvokeArgs.trim() ? JSON.parse(fnInvokeArgs) : {{}};
          const res = await fetch(`${{API}}/api/fn/${{fnInvokeName}}`, {{
            method: "POST",
            headers: {{ "Content-Type": "application/json" }},
            body: JSON.stringify(args),
          }});
          const data = await res.json();
          setFnInvokeResult({{ status: res.status, data }});
          loadFns();
        }} catch (err) {{
          setFnInvokeResult({{ status: 0, data: {{ error: err.message }} }});
        }}
      }};

      React.useEffect(() => {{
        if (tab === "functions") {{
          loadFns();
          const id = setInterval(loadFns, 5000);
          return () => clearInterval(id);
        }}
      }}, [tab]);

      const loadRows = async (e) => {{
        const target = e || entity;
        if (!target) return;
        setLoading(true);
        setError(null);
        try {{
          const res = await fetch(`${{API}}/api/entities/${{target}}`);
          const data = await res.json();
          setRows(data.data || data);
        }} catch (err) {{
          setError(err.message);
          setRows([]);
        }}
        setLoading(false);
      }};

      const insertRow = async () => {{
        try {{
          const data = JSON.parse(insertJson);
          const res = await fetch(`${{API}}/api/entities/${{entity}}`, {{
            method: "POST",
            headers: {{ "Content-Type": "application/json" }},
            body: JSON.stringify(data),
          }});
          const result = await res.json();
          if (result.error) {{ setError(result.error.message); return; }}
          setShowInsert(false);
          setInsertJson("{{}}");
          loadRows();
        }} catch (err) {{
          setError(err.message);
        }}
      }};

      const deleteRow = async (id) => {{
        await fetch(`${{API}}/api/entities/${{entity}}/${{id}}`, {{ method: "DELETE" }});
        loadRows();
      }};

      React.useEffect(() => {{ if (entity) loadRows(); }}, [entity]);

      // Connect WebSocket for live sync log + audit log.
      React.useEffect(() => {{
        try {{
          const url = new URL(API);
          const wsPort = parseInt(url.port || "4321") + 1;
          const ws = new WebSocket(`ws://${{url.hostname}}:${{wsPort}}`);
          ws.onmessage = (e) => {{
            try {{
              const msg = JSON.parse(e.data);
              if (msg.seq) {{
                setSyncLog(prev => [msg, ...prev].slice(0, 50));
                setAuditLog(prev => [{{
                  ...msg,
                  timestamp: new Date().toISOString(),
                }}, ...prev].slice(0, 200));
                if (msg.entity === entity) loadRows();
              }}
            }} catch {{}}
          }};
          return () => ws.close();
        }} catch {{}}
      }}, [entity]);

      // Load users when Users tab is active.
      const loadUsers = async () => {{
        setUsersLoading(true);
        try {{
          const res = await fetch(`${{API}}/api/entities/User`);
          const data = await res.json();
          setUsers(data.data || data);
        }} catch (err) {{
          setError(err.message);
          setUsers([]);
        }}
        setUsersLoading(false);
      }};

      React.useEffect(() => {{
        if (tab === "users") loadUsers();
      }}, [tab]);

      const createSession = async (userId) => {{
        try {{
          const res = await fetch(`${{API}}/api/auth/session`, {{
            method: "POST",
            headers: {{ "Content-Type": "application/json" }},
            body: JSON.stringify({{ user_id: userId }}),
          }});
          const data = await res.json();
          if (data.error) {{
            setError(data.error.message || data.error);
          }} else {{
            setSessionMsg(`Session created for ${{userId}}: ${{data.token || JSON.stringify(data)}}`);
            setTimeout(() => setSessionMsg(null), 8000);
          }}
        }} catch (err) {{
          setError(err.message);
        }}
      }};

      // Load rooms when Rooms tab is active, auto-refresh every 5s.
      const loadRooms = async () => {{
        setRoomsLoading(true);
        try {{
          const res = await fetch(`${{API}}/api/rooms`);
          const data = await res.json();
          setRooms(data.data || data);
        }} catch (err) {{
          setError(err.message);
          setRooms([]);
        }}
        setRoomsLoading(false);
      }};

      const loadRoomMembers = async (roomName) => {{
        setRoomMembersLoading(true);
        try {{
          const res = await fetch(`${{API}}/api/rooms/${{encodeURIComponent(roomName)}}`);
          const data = await res.json();
          setRoomMembers(data.members || data.data || data);
        }} catch (err) {{
          setError(err.message);
          setRoomMembers([]);
        }}
        setRoomMembersLoading(false);
      }};

      React.useEffect(() => {{
        if (tab === "rooms") {{
          loadRooms();
          const interval = setInterval(loadRooms, 5000);
          return () => clearInterval(interval);
        }}
      }}, [tab]);

      React.useEffect(() => {{
        if (expandedRoom) loadRoomMembers(expandedRoom);
      }}, [expandedRoom]);

      // Load health + metrics when Health tab is active, auto-refresh every 10s.
      const loadHealth = async () => {{
        setHealthLoading(true);
        try {{
          const [healthRes, metricsRes] = await Promise.all([
            fetch(`${{API}}/health`),
            fetch(`${{API}}/metrics`),
          ]);
          const healthData = await healthRes.json();
          setHealth(healthData);
          try {{
            const metricsData = await metricsRes.json();
            setMetrics(metricsData);
          }} catch {{
            // metrics may return non-JSON, handle gracefully
            setMetrics(null);
          }}
        }} catch (err) {{
          setError(err.message);
        }}
        setHealthLoading(false);
      }};

      React.useEffect(() => {{
        if (tab === "health") {{
          loadHealth();
          const interval = setInterval(loadHealth, 10000);
          return () => clearInterval(interval);
        }}
      }}, [tab]);

      // Filtered users based on search.
      const filteredUsers = React.useMemo(() => {{
        if (!usersSearch.trim()) return users;
        const q = usersSearch.toLowerCase();
        return users.filter(u =>
          (u.id && String(u.id).toLowerCase().includes(q)) ||
          (u.email && u.email.toLowerCase().includes(q)) ||
          (u.displayName && u.displayName.toLowerCase().includes(q)) ||
          (u.display_name && u.display_name.toLowerCase().includes(q))
        );
      }}, [users, usersSearch]);

      // Filtered audit log based on entity filter.
      const filteredAudit = React.useMemo(() => {{
        if (!auditEntityFilter) return auditLog;
        return auditLog.filter(e => e.entity === auditEntityFilter);
      }}, [auditLog, auditEntityFilter]);

      return (
        <div className="max-w-6xl mx-auto px-4 py-6">
          <div className="flex items-center justify-between mb-6">
            <div>
              <h1 className="text-xl font-semibold">pylon Studio</h1>
              <p className="text-sm text-muted-foreground">{name} v{version}</p>
            </div>
            <div className="flex gap-2">
              <span className="badge badge-secondary">{entity_count} entities</span>
              <span className="badge badge-secondary">{query_count} queries</span>
              <span className="badge badge-secondary">{action_count} actions</span>
              <span className="badge badge-secondary">{route_count} routes</span>
            </div>
          </div>

          <div className="flex gap-1 mb-4 border-b border-border overflow-x-auto">
            {{["entities", "users", "rooms", "audit", "health", "functions", "queries", "actions", "policies", "routes", "sync", "manifest"].map(t => (
              <div key={{t}} className={{`tab ${{tab === t ? "active" : ""}}`}} onClick={{() => setTab(t)}}>
                {{t.charAt(0).toUpperCase() + t.slice(1)}}
              </div>
            ))}}
          </div>

          {{error && (
            <div className="bg-destructive/10 text-destructive border border-destructive/20 rounded-md p-3 mb-4 text-sm">
              {{error}}
              <button className="ml-2 underline" onClick={{() => setError(null)}}>dismiss</button>
            </div>
          )}}

          {{/* ---- Entities Tab ---- */}}
          {{tab === "entities" && (
            <div>
              <div className="flex gap-2 mb-4">
                <select className="input" style={{{{width:'auto'}}}} value={{entity}} onChange={{(e) => {{ setEntity(e.target.value); }}}}>
                  {{ENTITIES.map(e => <option key={{e}} value={{e}}>{{e}}</option>)}}
                </select>
                <button className="btn btn-secondary" onClick={{() => loadRows()}}>Refresh</button>
                <button className="btn btn-primary" onClick={{() => setShowInsert(!showInsert)}}>
                  {{showInsert ? "Cancel" : "+ Insert"}}
                </button>
              </div>

              {{showInsert && (
                <div className="card p-4 mb-4">
                  <p className="text-sm text-muted-foreground mb-2">Insert JSON into {{entity}}:</p>
                  <textarea
                    className="input font-mono"
                    rows={{4}}
                    value={{insertJson}}
                    onChange={{(e) => setInsertJson(e.target.value)}}
                  />
                  <button className="btn btn-primary mt-2" onClick={{insertRow}}>Insert</button>
                </div>
              )}}

              {{loading ? (
                <p className="text-muted-foreground text-sm">Loading...</p>
              ) : rows.length === 0 ? (
                <p className="text-muted-foreground text-sm py-8 text-center">No rows in {{entity}}.</p>
              ) : (
                <div className="card overflow-hidden">
                  <table>
                    <thead>
                      <tr>
                        {{Object.keys(rows[0] || {{}}).map(k => <th key={{k}}>{{k}}</th>)}}
                        <th></th>
                      </tr>
                    </thead>
                    <tbody>
                      {{rows.map((row, i) => (
                        <tr key={{row.id || i}}>
                          {{Object.values(row).map((v, j) => (
                            <td key={{j}} className="font-mono text-xs">{{String(v)}}</td>
                          ))}}
                          <td>
                            <button className="btn btn-ghost text-xs" onClick={{() => deleteRow(row.id)}}>Delete</button>
                          </td>
                        </tr>
                      ))}}
                    </tbody>
                  </table>
                </div>
              )}}
            </div>
          )}}

          {{/* ---- Users Tab ---- */}}
          {{tab === "users" && (
            <div>
              <div className="flex gap-2 mb-4 items-center">
                <input
                  className="input"
                  style={{{{maxWidth: '320px'}}}}
                  placeholder="Search by id, email, or name..."
                  value={{usersSearch}}
                  onChange={{(e) => setUsersSearch(e.target.value)}}
                />
                <button className="btn btn-secondary" onClick={{loadUsers}}>Refresh</button>
                <span className="text-xs text-muted-foreground ml-auto">
                  {{filteredUsers.length}} user{{filteredUsers.length !== 1 ? "s" : ""}}
                </span>
              </div>

              {{sessionMsg && (
                <div className="bg-green-900/30 text-green-400 border border-green-800 rounded-md p-3 mb-4 text-sm font-mono break-all">
                  {{sessionMsg}}
                </div>
              )}}

              {{usersLoading ? (
                <p className="text-muted-foreground text-sm">Loading users...</p>
              ) : filteredUsers.length === 0 ? (
                <p className="text-muted-foreground text-sm py-8 text-center">No users found.</p>
              ) : (
                <div className="card overflow-hidden">
                  <table>
                    <thead>
                      <tr>
                        <th>id</th>
                        <th>email</th>
                        <th>displayName</th>
                        <th></th>
                      </tr>
                    </thead>
                    <tbody>
                      {{filteredUsers.map((u, i) => (
                        <tr key={{u.id || i}}>
                          <td className="font-mono text-xs">{{u.id || "—"}}</td>
                          <td className="text-xs">{{u.email || "—"}}</td>
                          <td className="text-xs">{{u.displayName || u.display_name || "—"}}</td>
                          <td>
                            <button className="btn btn-secondary text-xs" onClick={{() => createSession(u.id)}}>
                              Create Session
                            </button>
                          </td>
                        </tr>
                      ))}}
                    </tbody>
                  </table>
                </div>
              )}}
            </div>
          )}}

          {{/* ---- Audit Tab ---- */}}
          {{tab === "audit" && (
            <div>
              <div className="flex gap-2 mb-4 items-center">
                <select
                  className="input"
                  style={{{{width: 'auto'}}}}
                  value={{auditEntityFilter}}
                  onChange={{(e) => setAuditEntityFilter(e.target.value)}}
                >
                  <option value="">All entities</option>
                  {{ENTITIES.map(e => <option key={{e}} value={{e}}>{{e}}</option>)}}
                </select>
                <span className="text-xs text-muted-foreground ml-auto">
                  {{filteredAudit.length}} event{{filteredAudit.length !== 1 ? "s" : ""}} (max 200)
                </span>
              </div>

              <p className="text-xs text-muted-foreground mb-4">
                Events captured from the live WebSocket sync feed. Open the Sync tab to see raw events.
              </p>

              {{filteredAudit.length === 0 ? (
                <p className="text-muted-foreground text-sm py-8 text-center">
                  No audit events yet. Make a change to see events appear.
                </p>
              ) : (
                <div className="card overflow-hidden">
                  <table>
                    <thead>
                      <tr>
                        <th>seq</th>
                        <th>kind</th>
                        <th>entity</th>
                        <th>row_id</th>
                        <th>timestamp</th>
                      </tr>
                    </thead>
                    <tbody>
                      {{filteredAudit.map((e, i) => (
                        <tr key={{`${{e.seq}}-${{i}}`}}>
                          <td className="font-mono text-xs">{{e.seq}}</td>
                          <td>
                            <span className={{kindBadgeClass(e.kind)}}>{{e.kind}}</span>
                          </td>
                          <td className="text-xs">{{e.entity}}</td>
                          <td className="font-mono text-xs">{{e.row_id}}</td>
                          <td className="text-xs text-muted-foreground">
                            {{e.timestamp ? new Date(e.timestamp).toLocaleTimeString() : "—"}}
                          </td>
                        </tr>
                      ))}}
                    </tbody>
                  </table>
                </div>
              )}}
            </div>
          )}}

          {{/* ---- Rooms Tab ---- */}}
          {{tab === "rooms" && (
            <div>
              <div className="flex gap-2 mb-4 items-center">
                <h2 className="text-sm font-medium">Active Rooms</h2>
                <button className="btn btn-secondary text-xs" onClick={{loadRooms}}>Refresh</button>
                <span className="text-xs text-muted-foreground ml-auto">
                  Auto-refreshes every 5s
                </span>
              </div>

              {{roomsLoading && rooms.length === 0 ? (
                <p className="text-muted-foreground text-sm">Loading rooms...</p>
              ) : !Array.isArray(rooms) || rooms.length === 0 ? (
                <p className="text-muted-foreground text-sm py-8 text-center">No active rooms.</p>
              ) : (
                <div className="space-y-2">
                  {{rooms.map((room, i) => {{
                    const roomName = typeof room === "string" ? room : (room.name || room.id || `room-${{i}}`);
                    const memberCount = room.member_count || room.members_count || room.members?.length;
                    const isExpanded = expandedRoom === roomName;
                    return (
                      <div key={{roomName}} className="card">
                        <div
                          className="p-3 flex items-center justify-between cursor-pointer hover:bg-zinc-800/50 rounded-lg"
                          onClick={{() => setExpandedRoom(isExpanded ? null : roomName)}}
                        >
                          <div className="flex items-center gap-2">
                            <span className="font-mono text-sm">{{roomName}}</span>
                            {{memberCount != null && (
                              <span className="badge badge-blue">{{memberCount}} member{{memberCount !== 1 ? "s" : ""}}</span>
                            )}}
                          </div>
                          <span className="text-muted-foreground text-xs">{{isExpanded ? "▲" : "▼"}}</span>
                        </div>
                        {{isExpanded && (
                          <div className="border-t border-border p-3">
                            {{roomMembersLoading ? (
                              <p className="text-xs text-muted-foreground">Loading members...</p>
                            ) : !Array.isArray(roomMembers) || roomMembers.length === 0 ? (
                              <p className="text-xs text-muted-foreground">No members in this room.</p>
                            ) : (
                              <div className="space-y-1">
                                {{roomMembers.map((m, mi) => (
                                  <div key={{mi}} className="text-xs font-mono py-1 px-2 rounded bg-zinc-900">
                                    {{typeof m === "string" ? m : (m.user_id || m.id || JSON.stringify(m))}}
                                  </div>
                                ))}}
                              </div>
                            )}}
                          </div>
                        )}}
                      </div>
                    );
                  }})}}
                </div>
              )}}
            </div>
          )}}

          {{/* ---- Health Tab ---- */}}
          {{tab === "health" && (
            <div>
              <div className="flex gap-2 mb-4 items-center">
                <h2 className="text-sm font-medium">Server Health</h2>
                <button className="btn btn-secondary text-xs" onClick={{loadHealth}}>Refresh</button>
                <span className="text-xs text-muted-foreground ml-auto">
                  Auto-refreshes every 10s
                </span>
              </div>

              {{healthLoading && !health ? (
                <p className="text-muted-foreground text-sm">Loading health data...</p>
              ) : (
                <div>
                  {{/* Status + Uptime */}}
                  <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4 mb-6">
                    <div className="card p-4">
                      <p className="text-xs text-muted-foreground mb-1">Status</p>
                      <p className="text-lg font-semibold">
                        {{health ? (
                          <span className={{health.status === "ok" || health.status === "healthy" ? "text-green-400" : "text-yellow-400"}}>
                            {{health.status || "unknown"}}
                          </span>
                        ) : "—"}}
                      </p>
                    </div>
                    <div className="card p-4">
                      <p className="text-xs text-muted-foreground mb-1">Uptime</p>
                      <p className="text-lg font-semibold font-mono">
                        {{health && health.uptime_seconds != null
                          ? formatUptime(health.uptime_seconds)
                          : health && health.uptime != null
                            ? formatUptime(health.uptime)
                            : "—"
                        }}
                      </p>
                    </div>
                    <div className="card p-4">
                      <p className="text-xs text-muted-foreground mb-1">Version</p>
                      <p className="text-lg font-semibold font-mono">
                        {{health && health.version ? health.version : "—"}}
                      </p>
                    </div>
                  </div>

                  {{/* Metrics */}}
                  {{metrics && (
                    <div>
                      <h3 className="text-sm font-medium mb-3">Metrics</h3>
                      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-6">
                        {{metrics.total_requests != null && (
                          <div className="card p-4">
                            <p className="text-xs text-muted-foreground mb-1">Total Requests</p>
                            <p className="text-2xl font-semibold font-mono">{{metrics.total_requests.toLocaleString()}}</p>
                          </div>
                        )}}
                        {{metrics.active_connections != null && (
                          <div className="card p-4">
                            <p className="text-xs text-muted-foreground mb-1">Active Connections</p>
                            <p className="text-2xl font-semibold font-mono">{{metrics.active_connections}}</p>
                          </div>
                        )}}
                        {{metrics.error_count != null && (
                          <div className="card p-4">
                            <p className="text-xs text-muted-foreground mb-1">Errors</p>
                            <p className="text-2xl font-semibold font-mono text-destructive">{{metrics.error_count}}</p>
                          </div>
                        )}}
                        {{metrics.avg_response_ms != null && (
                          <div className="card p-4">
                            <p className="text-xs text-muted-foreground mb-1">Avg Response</p>
                            <p className="text-2xl font-semibold font-mono">{{metrics.avg_response_ms}}ms</p>
                          </div>
                        )}}
                      </div>

                      {{/* Method breakdown */}}
                      {{metrics.by_method && typeof metrics.by_method === "object" && (
                        <div>
                          <h3 className="text-sm font-medium mb-3">Requests by Method</h3>
                          <div className="card overflow-hidden">
                            <table>
                              <thead>
                                <tr>
                                  <th>Method</th>
                                  <th>Count</th>
                                </tr>
                              </thead>
                              <tbody>
                                {{Object.entries(metrics.by_method).map(([method, count]) => (
                                  <tr key={{method}}>
                                    <td>
                                      <span className="badge badge-secondary font-mono">{{method}}</span>
                                    </td>
                                    <td className="font-mono text-xs">{{typeof count === "number" ? count.toLocaleString() : String(count)}}</td>
                                  </tr>
                                ))}}
                              </tbody>
                            </table>
                          </div>
                        </div>
                      )}}

                      {{/* Fallback: render all metric keys as cards if none of the expected fields matched */}}
                      {{metrics.total_requests == null && metrics.by_method == null && (
                        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
                          {{Object.entries(metrics).map(([key, value]) => (
                            <div key={{key}} className="card p-4">
                              <p className="text-xs text-muted-foreground mb-1">{{key}}</p>
                              <p className="text-sm font-mono break-all">{{typeof value === "object" ? JSON.stringify(value) : String(value)}}</p>
                            </div>
                          ))}}
                        </div>
                      )}}
                    </div>
                  )}}
                </div>
              )}}
            </div>
          )}}

          {{/* ---- Sync Tab ---- */}}
          {{tab === "sync" && (
            <div>
              <h2 className="text-sm font-medium mb-2">Live Sync Events</h2>
              <p className="text-xs text-muted-foreground mb-4">Connected via WebSocket. Events appear in real-time.</p>
              {{syncLog.length === 0 ? (
                <p className="text-muted-foreground text-sm py-8 text-center">No events yet. Make a change to see sync events.</p>
              ) : (
                <div className="space-y-2">
                  {{syncLog.map((e, i) => (
                    <div key={{i}} className="card p-3 text-xs font-mono">
                      <span className="text-muted-foreground">[{{e.seq}}]</span>
                      {{" "}}
                      <span className={{e.kind === "delete" ? "text-destructive" : e.kind === "insert" ? "text-green-400" : "text-yellow-400"}}>
                        {{e.kind}}
                      </span>
                      {{" "}}{{e.entity}}/{{e.row_id}}
                    </div>
                  ))}}
                </div>
              )}}
            </div>
          )}}

          {{/* ---- Functions Tab ---- */}}
          {{tab === "functions" && (
            <div>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4">
                <div className="card p-4">
                  <div className="flex items-center justify-between mb-3">
                    <h2 className="text-sm font-medium">Registered ({{fns.length}})</h2>
                    <button className="btn btn-ghost text-xs" onClick={{loadFns}}>Refresh</button>
                  </div>
                  {{fnsLoading ? (
                    <p className="text-muted-foreground text-sm">Loading...</p>
                  ) : fns.length === 0 ? (
                    <p className="text-muted-foreground text-sm">No TypeScript functions registered. Add files to ./functions/ and restart.</p>
                  ) : (
                    <div>
                      {{fns.map(f => (
                        <div key={{f.name}} className="py-1.5 border-b border-border last:border-0 flex items-center justify-between">
                          <div>
                            <span className="font-mono text-sm">{{f.name}}</span>
                            <span className={{`badge ml-2 ${{f.fn_type === 'mutation' ? 'badge-yellow' : f.fn_type === 'query' ? 'badge-blue' : 'badge-purple'}}`}}>{{f.fn_type}}</span>
                          </div>
                          <button className="btn btn-ghost text-xs" onClick={{() => {{ setFnInvokeName(f.name); setFnInvokeArgs("{{}}"); setFnInvokeResult(null); }}}}>Invoke</button>
                        </div>
                      ))}}
                    </div>
                  )}}
                </div>

                <div className="card p-4">
                  <h2 className="text-sm font-medium mb-3">Invoke</h2>
                  <input
                    className="input mb-2 font-mono"
                    placeholder="function name"
                    value={{fnInvokeName}}
                    onChange={{(e) => setFnInvokeName(e.target.value)}}
                  />
                  <textarea
                    className="input font-mono mb-2"
                    rows={{4}}
                    placeholder='{{ "name": "value" }}'
                    value={{fnInvokeArgs}}
                    onChange={{(e) => setFnInvokeArgs(e.target.value)}}
                  />
                  <button className="btn btn-primary" onClick={{invokeFn}} disabled={{!fnInvokeName}}>
                    Call
                  </button>
                  {{fnInvokeResult && (
                    <div className="mt-3">
                      <p className="text-xs text-muted-foreground mb-1">Status: {{fnInvokeResult.status}}</p>
                      <pre><code>{{JSON.stringify(fnInvokeResult.data, null, 2)}}</code></pre>
                    </div>
                  )}}
                </div>
              </div>

              <div className="card p-4">
                <h2 className="text-sm font-medium mb-3">Recent traces ({{fnTraces.length}})</h2>
                {{fnTraces.length === 0 ? (
                  <p className="text-muted-foreground text-sm">No traces yet — invoke a function to see one here.</p>
                ) : (
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div>
                      {{fnTraces.map((t, i) => (
                        <div
                          key={{i}}
                          className={{`py-1.5 px-2 cursor-pointer rounded ${{selectedTrace === t ? 'bg-accent' : ''}} hover:bg-accent`}}
                          onClick={{() => setSelectedTrace(t)}}
                        >
                          <div className="flex items-center justify-between text-sm">
                            <span className="font-mono">{{t.fn_name || t.name}}</span>
                            <span className={{`badge ${{t.error ? 'badge-red' : 'badge-green'}}`}}>
                              {{t.error ? "error" : "ok"}}
                            </span>
                          </div>
                          <div className="text-xs text-muted-foreground">
                            {{t.duration_ms !== undefined ? `${{t.duration_ms}}ms` : ""}}
                            {{t.timestamp ? ` · ${{new Date(t.timestamp).toLocaleTimeString()}}` : ""}}
                          </div>
                        </div>
                      ))}}
                    </div>
                    <div>
                      {{selectedTrace ? (
                        <pre><code>{{JSON.stringify(selectedTrace, null, 2)}}</code></pre>
                      ) : (
                        <p className="text-muted-foreground text-sm">Click a trace to see details.</p>
                      )}}
                    </div>
                  </div>
                )}}
              </div>
            </div>
          )}}

          {{/* ---- Manifest Tab ---- */}}
          {{tab === "manifest" && (
            <div>
              <pre><code>{{JSON.stringify(MANIFEST, null, 2)}}</code></pre>
            </div>
          )}}

          {{/* ---- Queries Tab ---- */}}
          {{tab === "queries" && (
            <div className="card p-4">
              <h2 className="text-sm font-medium mb-3">Queries</h2>
              {{MANIFEST.queries.map(q => (
                <div key={{q.name}} className="py-2 border-b border-border last:border-0">
                  <span className="font-mono text-sm">{{q.name}}</span>
                  {{q.input && q.input.length > 0 && (
                    <span className="text-xs text-muted-foreground ml-2">
                      ({{q.input.map(f => `${{f.name}}: ${{f.type}}`).join(", ")}})
                    </span>
                  )}}
                </div>
              ))}}
            </div>
          )}}

          {{/* ---- Actions Tab ---- */}}
          {{tab === "actions" && (
            <div className="card p-4">
              <h2 className="text-sm font-medium mb-3">Actions</h2>
              {{MANIFEST.actions.map(a => (
                <div key={{a.name}} className="py-2 border-b border-border last:border-0">
                  <span className="font-mono text-sm">{{a.name}}</span>
                  {{a.input && a.input.length > 0 && (
                    <span className="text-xs text-muted-foreground ml-2">
                      ({{a.input.map(f => `${{f.name}}: ${{f.type}}`).join(", ")}})
                    </span>
                  )}}
                </div>
              ))}}
            </div>
          )}}

          {{/* ---- Policies Tab ---- */}}
          {{tab === "policies" && (
            <div className="card p-4">
              <h2 className="text-sm font-medium mb-3">Policies</h2>
              {{MANIFEST.policies.map(p => (
                <div key={{p.name}} className="py-2 border-b border-border last:border-0">
                  <span className="font-mono text-sm">{{p.name}}</span>
                  <span className="text-xs text-muted-foreground ml-2">
                    {{p.entity && `entity=${{p.entity}} `}}
                    {{p.action && `action=${{p.action}} `}}
                    → {{p.allow}}
                  </span>
                </div>
              ))}}
            </div>
          )}}

          {{/* ---- Routes Tab ---- */}}
          {{tab === "routes" && (
            <div className="card p-4">
              <h2 className="text-sm font-medium mb-3">Routes</h2>
              {{MANIFEST.routes.map(r => (
                <div key={{r.path}} className="py-2 border-b border-border last:border-0 flex items-center gap-2">
                  <span className="font-mono text-sm">{{r.path}}</span>
                  <span className="badge badge-secondary">{{r.mode}}</span>
                  {{r.query && <span className="text-xs text-muted-foreground">query={{r.query}}</span>}}
                  {{r.auth && <span className="text-xs text-muted-foreground">auth={{r.auth}}</span>}}
                </div>
              ))}}
            </div>
          )}}
        </div>
      );
    }}

    ReactDOM.createRoot(document.getElementById("root")).render(<App />);
  </script>
</body>
</html>"##,
        name = name_safe,
        version = version_safe,
        api_base = api_base_safe,
        manifest_json = manifest_json,
        entity_json = entity_json,
        entity_count = manifest.entities.len(),
        query_count = manifest.queries.len(),
        action_count = manifest.actions.len(),
        route_count = manifest.routes.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest() -> AppManifest {
        serde_json::from_str(include_str!(
            "../../../examples/todo-app/pylon.manifest.json"
        ))
        .unwrap()
    }

    #[test]
    fn generates_html() {
        let html = generate_studio_html(&test_manifest(), "http://localhost:4321");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("pylon Studio"));
        assert!(html.contains("todo-app"));
        assert!(html.contains("tailwindcss"));
        assert!(html.contains("react"));
    }

    #[test]
    fn html_escapes_manifest_name_in_title() {
        // The <title> tag interpolates manifest.name as raw HTML. An
        // attacker-influenced name must not break out of the title element.
        let mut m = test_manifest();
        m.name = "</title><script>alert('x')</script>".into();
        let html = generate_studio_html(&m, "http://localhost:4321");

        // Extract the title text and verify it only contains the escaped
        // form. (The full HTML contains the manifest_json embed too, which
        // carries the raw-looking name inside a <script> block — that path
        // is safe because </script is independently escaped.)
        let start = html.find("<title>").expect("no <title>");
        let end = html[start..].find("</title>").expect("no </title>");
        let title = &html[start..start + end];
        assert!(
            !title.contains("<script>"),
            "XSS: raw <script> tag inside <title>: {title:?}"
        );
        assert!(title.contains("&lt;/title&gt;"));
    }

    #[test]
    fn html_escapes_manifest_version() {
        // version is embedded inside JSX text via format!. Even though React
        // escapes text children, the pre-Babel format! substitution must
        // HTML-encode so the substituted bytes can't alter the JSX AST.
        let mut m = test_manifest();
        m.version = "<img src=x onerror=alert(1)>".into();
        let html = generate_studio_html(&m, "http://localhost:4321");
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    }

    #[test]
    fn api_base_is_js_escaped() {
        // api_base is interpolated into `const API = "…";` as a JS string
        // literal. A quote in the input must appear backslash-escaped
        // (or Unicode-escaped) in the output so it can't close the string
        // early.
        let m = test_manifest();
        let html = generate_studio_html(&m, "http://example.com\"; alert(1); //");
        // Locate the exact `const API = "..."` statement.
        let needle = "const API = \"";
        let start = html.find(needle).expect("no const API line");
        let rest = &html[start + needle.len()..];
        // Find the end of the JS string literal: first unescaped ".
        let mut idx = 0;
        let bytes = rest.as_bytes();
        while idx < bytes.len() {
            if bytes[idx] == b'"' {
                // Check if escaped by backslash. Walk back counting \.
                let mut backslashes = 0usize;
                let mut j = idx;
                while j > 0 && bytes[j - 1] == b'\\' {
                    backslashes += 1;
                    j -= 1;
                }
                if backslashes % 2 == 0 {
                    break;
                }
            }
            idx += 1;
        }
        let literal = &rest[..idx];
        assert!(
            !literal.contains("; alert(1); //")
                || literal.contains("\\\"; alert(1); //")
                || literal.contains("\\u0022"),
            "XSS: raw quote broke out of JS string: {literal:?}"
        );
    }

    #[test]
    fn escape_script_json_is_case_insensitive() {
        // Regression: mixed-case </ScRiPt> used to slip past the filter
        // and close the <script> tag that surrounds the embedded MANIFEST
        // JSON. Any casing must now get the backslash break.
        let mut m = test_manifest();
        m.entities[0].name = "E</ScRiPt><svg onload=alert(1)>".into();
        let html = generate_studio_html(&m, "http://ok");
        // Walk the HTML looking for a closing script tag embedded in a
        // string context. Any `</script` in any case in the manifest JSON
        // would be a break-out; the escape replaces it with `<\\/script`
        // (JSON backslash followed by forward slash).
        let embedded = find_manifest_json_block(&html);
        let lower = embedded.to_ascii_lowercase();
        // Look for literal "</script" in the embedded JSON. After fix,
        // every occurrence must be preceded by a backslash.
        let mut pos = 0;
        while let Some(idx) = lower[pos..].find("</script") {
            let abs = pos + idx;
            let preceded_by_backslash = abs > 0 && embedded.as_bytes()[abs - 1] == b'\\';
            assert!(
                preceded_by_backslash,
                "unescaped </script at byte {abs} in: {embedded}"
            );
            pos = abs + 1;
        }
    }

    #[test]
    fn escape_script_json_handles_line_separator() {
        // U+2028 / U+2029 close JS string literals in Babel's parser.
        // Escape them so a crafted name can't break the MANIFEST const.
        let mut m = test_manifest();
        m.entities[0].name = "ok\u{2028}oops".into();
        let html = generate_studio_html(&m, "http://ok");
        let embedded = find_manifest_json_block(&html);
        assert!(
            !embedded.contains('\u{2028}'),
            "U+2028 leaked into the embedded manifest JSON"
        );
        assert!(embedded.contains("\\u2028"));
    }

    fn find_manifest_json_block(html: &str) -> String {
        // The MANIFEST const in the generated HTML sits inline:
        //   const MANIFEST = {...};
        let start = html.find("const MANIFEST = ").expect("no MANIFEST const");
        let after = &html[start..];
        let end = after
            .find(";\n")
            .unwrap_or_else(|| after.len().min(100_000));
        after[..end].to_string()
    }

    #[test]
    fn escape_helpers_directly() {
        // Unit-test the helpers so future edits are harder to get wrong.
        // (Defined inside generate_studio_html, so exercise through the
        // public function.)
        let mut m = test_manifest();
        m.name = "A&B <C>".into();
        let html = generate_studio_html(&m, "http://ok");
        let start = html.find("<title>").unwrap();
        let end = html[start..].find("</title>").unwrap();
        assert!(html[start..start + end].contains("A&amp;B &lt;C&gt;"));
    }

    #[test]
    fn includes_entity_data() {
        let html = generate_studio_html(&test_manifest(), "http://localhost:4321");
        assert!(html.contains("User"));
        assert!(html.contains("Todo"));
    }

    #[test]
    fn includes_manifest() {
        let html = generate_studio_html(&test_manifest(), "http://localhost:4321");
        assert!(html.contains("manifest_version"));
        assert!(html.contains("todosByAuthor"));
    }

    #[test]
    fn includes_dark_theme() {
        let html = generate_studio_html(&test_manifest(), "http://localhost:4321");
        assert!(html.contains("dark"));
        assert!(html.contains("#09090b")); // zinc-950
    }

    #[test]
    fn includes_websocket_sync() {
        let html = generate_studio_html(&test_manifest(), "http://localhost:4321");
        assert!(html.contains("WebSocket"));
        assert!(html.contains("syncLog"));
    }
}
