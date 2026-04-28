/** @type {import('next').NextConfig} */
const nextConfig = {
  // Proxy /api/* to the Pylon backend so the browser can use the
  // session cookie without CORS, and so client-side `@pylonsync/react`
  // hooks talk to a same-origin URL.
  async rewrites() {
    const target = process.env.PYLON_TARGET ?? "http://localhost:4321";
    return [
      { source: "/api/:path*", destination: `${target}/api/:path*` },
    ];
  },
};

module.exports = nextConfig;
