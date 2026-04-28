/// <reference path="./.sst/platform/config.d.ts" />

// Reference SST v3 config for deploying Pylon to AWS.
// See https://docs.pylonsync.com/operations/sst for the walkthrough.
//
// Provisions (the production-ready default):
//   - Aurora Serverless v2 Postgres for app data + sessions
//   - S3 bucket for file uploads (linked to the service for IAM)
//   - ECS Fargate service running the Pylon container (stateless)
//   - ALB with HTTP + WebSocket + SSE + shard ports forwarded
//   - Secrets via AWS Secrets Manager
//   - CloudFront CDN in front of the ALB
//
// This shape is horizontally scalable: the container holds no state,
// so you can bump `scaling.min/max` without losing data on replica churn.
//
// Before first deploy:
//   openssl rand -hex 32 | xargs sst secret set PylonAdminToken
//   sst secret set OAuthGoogleClientSecret    # if using Google OAuth
//   sst secret set OAuthGithubClientSecret    # if using GitHub OAuth
//
// Then:
//   sst deploy --stage production

export default $config({
  app(input) {
    return {
      name: "pylon",
      removal: input?.stage === "production" ? "retain" : "remove",
      home: "aws",
      providers: { aws: { region: "us-east-1" } },
    };
  },
  async run() {
    // ── Secrets ──────────────────────────────────────────────────────
    const adminToken = new sst.Secret("PylonAdminToken");
    const oauthGoogle = new sst.Secret("OAuthGoogleClientSecret");
    const oauthGithub = new sst.Secret("OAuthGithubClientSecret");

    // ── Database ─────────────────────────────────────────────────────
    // Aurora Serverless v2 — auto-scales 0.5 → 2 ACU. Holds app data
    // AND sessions (Pylon's session store writes to the same Postgres
    // when DATABASE_URL is set and PYLON_SESSION_DB is unset).
    const db = new sst.aws.Postgres("PylonDb", {
      scaling: { min: "0.5 ACU", max: "2 ACU" },
    });

    // ── File storage (S3) ────────────────────────────────────────────
    // The container is stateless; uploads land in S3 via the
    // file_storage plugin. `link` grants the service IAM permissions
    // (PutObject / GetObject / DeleteObject) on this bucket
    // automatically — no manual policy attachment needed.
    const uploads = new sst.aws.Bucket("PylonUploads");

    // ── Cluster + service ────────────────────────────────────────────
    const cluster = new sst.aws.Cluster("PylonCluster", {
      vpc: { id: db.nodes.vpc.id },
    });

    const service = new sst.aws.Service("PylonService", {
      cluster,
      cpu: "0.25 vCPU",
      memory: "512 MB",
      image: { dockerfile: "../../Dockerfile", context: "../.." },
      health: {
        command: [
          "CMD-SHELL",
          "curl -fsS http://localhost:4321/health || exit 1",
        ],
        interval: "30 seconds",
      },
      // Bump min/max to scale horizontally — safe because state lives
      // in Postgres + S3, not on the container's filesystem.
      scaling: { min: 1, max: 4, cpuUtilization: 70 },
      link: [uploads],
      environment: {
        // Core
        DATABASE_URL: db.url,
        PYLON_PORT: "4321",
        PYLON_DEV_MODE: "false",
        PYLON_MANIFEST: "/app/pylon.manifest.json",
        // File storage — read by the file_storage plugin in your manifest.
        PYLON_FILES_PROVIDER: "s3",
        PYLON_S3_BUCKET: uploads.name,
        PYLON_S3_REGION: "us-east-1",
        // Auth (secrets)
        PYLON_ADMIN_TOKEN: adminToken.value,
        // Client-facing
        PYLON_CORS_ORIGIN: "https://your-app.com",
        PYLON_CSRF_ORIGINS: "https://your-app.com",
        // OAuth (optional — set the client IDs to your registered values)
        PYLON_OAUTH_GOOGLE_CLIENT_ID: "your-google-client-id",
        PYLON_OAUTH_GOOGLE_CLIENT_SECRET: oauthGoogle.value,
        PYLON_OAUTH_GITHUB_CLIENT_ID: "your-github-client-id",
        PYLON_OAUTH_GITHUB_CLIENT_SECRET: oauthGithub.value,
      },
      loadBalancer: {
        // ALB rules forward all four Pylon ports:
        //   4321 → HTTP API
        //   4322 → WebSocket sync (real-time CRDT + change events)
        //   4323 → SSE fallback (/events)
        //   4324 → realtime shards
        // The container's Dockerfile must EXPOSE all four; update the
        // shipped Dockerfile if you've changed PYLON_PORT.
        rules: [
          { listen: "443/https", forward: "4321/http" },
          { listen: "4322/tcp", forward: "4322/tcp" },
          { listen: "4323/tcp", forward: "4323/tcp" },
          { listen: "4324/tcp", forward: "4324/tcp" },
        ],
        // ACM cert provisioned automatically.
        domain: { name: "api.your-app.com", dns: sst.aws.dns() },
        // Bump idle timeout for long-lived WebSocket connections.
        idleTimeout: "3600 seconds",
        // Sticky sessions so a reconnecting WebSocket lands on the same
        // replica that holds its presence state. Required when
        // scaling.max > 1.
        stickySessions: true,
      },
    });

    // ── CDN ──────────────────────────────────────────────────────────
    // Caches static GET responses; WebSocket / SSE traffic should bypass
    // the CDN by pointing wsUrl directly at the ALB.
    const cdn = new sst.aws.Cdn("PylonCdn", {
      origins: [{ domainName: service.url }],
    });

    return {
      apiUrl: service.url,
      cdnUrl: cdn.url,
      dbHost: db.host,
      uploadsBucket: uploads.name,
    };
  },
});
