const STEPS = [
  {
    num: "01",
    title: "Local",
    meta: "pylon dev",
    desc: "SQLite backend, hot-reload, type-safe client regen. Zero config while you build.",
    tag: { label: "SQLite", accent: false },
  },
  {
    num: "02",
    title: "Pylon Cloud",
    meta: "managed",
    desc: "Start with hosted infra when you want the framework, not another operations project.",
    href: "https://cloud.pylonsync.com",
    tag: { label: "managed", accent: true },
  },
  {
    num: "03",
    title: "Your infra",
    meta: "docker · systemd",
    desc: "Run the same app on a VPS, container platform, or private network when control matters.",
    tag: { label: "portable", accent: false },
  },
  {
    num: "04",
    title: "AWS ECS + Aurora",
    meta: "terraform apply",
    desc: "Move into your AWS account with Postgres, load balancing, secrets, and your VPC.",
    tag: { label: "terraform", accent: false },
  },
];

export function Scale() {
  return (
    <section className="section" id="scale">
      <div className="container-page">
        <div className="section-label">Scales with you</div>
        <h2 className="section-title">
          Start managed.
          <br />
          Keep the escape hatch.
        </h2>
        <p className="section-sub">
          Pylon is not a hosting bet. It is one app model that can run locally,
          on Pylon Cloud, on a VPS, or inside your AWS account without rewriting
          the handlers that make your product work.
        </p>

        <div className="stepper">
          {STEPS.map((s, i) => (
            <div className="step" key={i}>
              <div className="step-num">{s.num}</div>
              <h3 className="step-title">
                {"href" in s ? (
                  <a href={s.href} target="_blank" rel="noopener noreferrer">
                    {s.title}
                  </a>
                ) : (
                  s.title
                )}
              </h3>
              <div className="step-meta">{s.meta}</div>
              <p className="step-desc">{s.desc}</p>
              <div className={`step-tag ${s.tag.accent ? "accent" : ""}`}>
                {s.tag.label}
              </div>
              {i < STEPS.length - 1 && (
                <div className="step-arrow">
                  <svg
                    width="10"
                    height="10"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2.2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <polyline points="9 6 15 12 9 18" />
                  </svg>
                </div>
              )}
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
