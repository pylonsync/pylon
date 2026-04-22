export function Footer() {
  return (
    <footer className="footer" id="status">
      <div className="container-page">
        <div className="section-label" style={{ marginBottom: 24 }}>
          Status &amp; license
        </div>
        <div
          style={{
            display: "flex",
            gap: 10,
            flexWrap: "wrap",
            marginBottom: 28,
          }}
        >
          <div className="status-pill warn">
            <span className="dot" /> pre-1.0 · breaking changes possible
          </div>
          <div className="status-pill">
            <span className="dot" /> MIT / Apache-2.0 dual-licensed
          </div>
          <div className="status-pill">
            <span className="dot" /> SOC 2 not yet — see{" "}
            <span style={{ color: "var(--accent)", marginLeft: 4 }}>
              SECURITY.md
            </span>
          </div>
        </div>

        <div className="footer-grid">
          <div>
            <div className="logo" style={{ marginBottom: 14 }}>
              <span className="logo-mark">▲</span>
              statecraft
            </div>
            <p
              style={{
                color: "var(--text-3)",
                fontSize: 13,
                lineHeight: 1.55,
                maxWidth: 340,
                margin: 0,
              }}
            >
              One Rust binary for real-time apps and games. Self-hostable. Open
              source. No managed tier, no lock-in.
            </p>
          </div>
          <div>
            <h5>Docs</h5>
            <ul>
              <li><a href="#">Getting started</a></li>
              <li><a href="#">Schema &amp; queries</a></li>
              <li><a href="#">Shards &amp; ticks</a></li>
              <li><a href="#">Deploy guides</a></li>
            </ul>
          </div>
          <div>
            <h5>Project</h5>
            <ul>
              <li><a href="#">GitHub</a></li>
              <li><a href="#">Changelog</a></li>
              <li><a href="#">Roadmap</a></li>
              <li><a href="#">SECURITY.md</a></li>
            </ul>
          </div>
          <div>
            <h5>Community</h5>
            <ul>
              <li><a href="#">Discord</a></li>
              <li><a href="#">Discussions</a></li>
              <li><a href="#">Contributing</a></li>
              <li><a href="#">Code of Conduct</a></li>
            </ul>
          </div>
        </div>

        <div className="footer-bottom">
          <div>© 2026 statecraft contributors · MIT / Apache-2.0</div>
          <div style={{ display: "flex", gap: 16 }}>
            <span>v0.8.2</span>
            <span>commit 4e8f2a1</span>
            <span style={{ color: "var(--green)" }}>● all systems nominal</span>
          </div>
        </div>
      </div>
    </footer>
  );
}
