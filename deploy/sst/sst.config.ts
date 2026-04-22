/// <reference path="./.sst/platform/config.d.ts" />

export default $config({
  app(input) {
    return {
      name: "statecraft",
      removal: input?.stage === "production" ? "retain" : "remove",
      home: "aws",
    };
  },
  async run() {
    // Aurora Serverless v2 PostgreSQL
    const db = new sst.aws.Postgres("AgentDB", {
      scaling: {
        min: "0.5 ACU",
        max: "2 ACU",
      },
    });

    // ECS Fargate service
    const cluster = new sst.aws.Cluster("AgentCluster", {
      vpc: { id: db.vpc.id },
    });

    const service = cluster.addService("AgentService", {
      cpu: "0.25 vCPU",
      memory: "512 MB",
      image: {
        dockerfile: "../../Dockerfile",
        context: "../../",
      },
      health: {
        path: "/health",
        interval: "30 seconds",
      },
      environment: {
        DATABASE_URL: db.url,
        STATECRAFT_PORT: "8080",
        STATECRAFT_DEV_MODE: "false",
        STATECRAFT_ADMIN_TOKEN: new sst.Secret("AdminToken").value,
      },
      public: {
        ports: [
          { listen: "80/http", forward: "8080/http" },
          { listen: "443/https", forward: "8080/http" },
        ],
      },
    });

    // CDN for caching static responses
    const cdn = new sst.aws.Cdn("AgentCdn", {
      origins: [
        {
          domainName: service.url,
        },
      ],
    });

    return {
      url: service.url,
      cdn: cdn.url,
      db: db.host,
    };
  },
});
