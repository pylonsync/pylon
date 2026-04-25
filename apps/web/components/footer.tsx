import { PylonMark } from "./pylon-logo";

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
              <PylonMark size={22} />
              <span>Pylon</span>
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
              <li><a href="https://docs.pylonsync.com">Introduction</a></li>
              <li><a href="https://docs.pylonsync.com/quickstart">Quickstart</a></li>
              <li><a href="https://docs.pylonsync.com/concepts/entities">Entities &amp; schema</a></li>
              <li><a href="https://docs.pylonsync.com/concepts/live-queries">Live queries</a></li>
              <li><a href="/skill">Claude Code skill</a></li>
            </ul>
          </div>
          <div>
            <h5>Project</h5>
            <ul>
              <li><a href="https://github.com/pylonsync/pylon">GitHub</a></li>
              <li><a href="https://github.com/pylonsync/pylon/releases">Changelog</a></li>
              <li><a href="https://github.com/pylonsync/pylon/blob/main/ROADMAP.md">Roadmap</a></li>
              <li><a href="https://github.com/pylonsync/pylon/blob/main/SECURITY.md">SECURITY.md</a></li>
            </ul>
          </div>
          <div>
            <h5>Community</h5>
            <ul>
              <li><a href="https://github.com/pylonsync/pylon/discussions">Discussions</a></li>
              <li><a href="https://github.com/pylonsync/pylon/blob/main/CONTRIBUTING.md">Contributing</a></li>
              <li><a href="https://github.com/pylonsync/pylon/blob/main/CODE_OF_CONDUCT.md">Code of Conduct</a></li>
            </ul>
          </div>
        </div>

        <div className="footer-bottom">
          <div>© 2026 pylon contributors · MIT / Apache-2.0</div>
          <div>Pre-1.0 · see GitHub for current release status</div>
        </div>
      </div>
    </footer>
  );
}
