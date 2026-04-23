const STEPS = [
  {
    num: "01",
    title: "Laptop",
    meta: "pylon dev",
    desc: "SQLite backend, hot-reload, type-safe client regen. Zero config to start.",
    tag: { label: "SQLite", accent: false },
  },
  {
    num: "02",
    title: "VPS",
    meta: "systemd · one binary",
    desc: "Ship the same binary to a $5 box. Built-in TLS, embedded storage, graceful restart.",
    tag: { label: "docker run", accent: false },
  },
  {
    num: "03",
    title: "Workers",
    meta: "cloudflare edge",
    desc: "Run on Cloudflare Workers with Durable Objects for shards. Scale-to-zero, global by default.",
    tag: { label: "scale-to-zero", accent: true },
  },
  {
    num: "04",
    title: "AWS ECS + Aurora",
    meta: "terraform apply",
    desc: "Included Terraform module: ECS services, Aurora Postgres, ALB, secrets. Your VPC, your keys.",
    tag: { label: "terraform", accent: false },
  },
];

export function Scale() {
  return (
    <section className="section" id="scale">
      <div className="container-page">
        <div className="section-label">Scales with you</div>
        <h2 className="section-title">
          Same binary. Same code.
          <br />
          Four deploy targets.
        </h2>
        <p className="section-sub">
          Start on a laptop, end on a managed cluster — without rewriting a handler.
          The storage driver is pluggable; everything above it is identical.
        </p>

        <div className="stepper">
          {STEPS.map((s, i) => (
            <div className="step" key={i}>
              <div className="step-num">{s.num}</div>
              <h3 className="step-title">{s.title}</h3>
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
