import { createRootRoute, Outlet, Scripts } from "@tanstack/react-router";
import { Meta } from "@tanstack/react-start";

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1" },
      { title: "__APP_NAME__" },
    ],
  }),
  component: RootComponent,
});

function RootComponent() {
  return (
    <html lang="en">
      <head>
        <Meta />
      </head>
      <body
        style={{
          margin: 0,
          padding: 0,
          fontFamily: "system-ui, -apple-system, sans-serif",
          background: "#fafafa",
          color: "#111",
        }}
      >
        <Outlet />
        <Scripts />
      </body>
    </html>
  );
}
