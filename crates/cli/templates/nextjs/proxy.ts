import { createPylonProxy } from "@pylonsync/next/proxy";

// Gate /dashboard/* on the presence of the Pylon session cookie.
// Forged values still fail server-side at pylon.requireAuth() in the
// dashboard layout — this just stops protected UI from flashing
// before the redirect.
const { proxy, config } = createPylonProxy({
  cookieName: "__APP_NAME___session",
  loginUrl: "/login",
  matcher: ["/dashboard/:path*"],
});

export { proxy, config };
