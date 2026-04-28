import { createFileRoute, redirect, useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { clearToken, getMe, pylonJson, type Me } from "../lib/pylon";

type Post = {
  id: string;
  title: string;
  slug: string;
  body?: string;
  publishedAt?: string | null;
};

export const Route = createFileRoute("/dashboard")({
  // beforeLoad runs on the server during SSR and on the client during
  // navigation. Redirecting from here means unauthenticated users never
  // see the protected UI flash.
  beforeLoad: async () => {
    const me = await getMe();
    if (!me) throw redirect({ to: "/login" });
    return { me };
  },
  component: DashboardPage,
});

function DashboardPage() {
  const { me } = Route.useRouteContext();
  const navigate = useNavigate();
  const [posts, setPosts] = useState<Post[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    pylonJson<Post[]>("/api/entities/Post")
      .then((data) => { setPosts(data); setError(null); })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, []);

  return (
    <div>
      <nav style={navStyle}>
        <strong>__APP_NAME__</strong>
        <div style={{ display: "flex", alignItems: "center", gap: "1rem" }}>
          <span style={{ color: "#666", fontSize: 14 }}>
            {me.user_id}
            {me.is_admin && <span style={badgeStyle}>admin</span>}
          </span>
          <button
            onClick={() => { clearToken(); navigate({ to: "/" }); }}
            style={signOutStyle}
          >
            Sign out
          </button>
        </div>
      </nav>
      <main style={{ maxWidth: 960, margin: "2rem auto", padding: "0 2rem" }}>
        <h1 style={{ marginTop: 0 }}>Dashboard</h1>
        <p style={{ color: "#666" }}>
          Posts loaded from <code>/api/entities/Post</code>. Add some via the API or{" "}
          <a href="http://localhost:4321/studio" style={{ color: "#0369a1" }}>Studio</a>.
        </p>

        {loading && <p style={{ color: "#666" }}>Loading…</p>}
        {error && <pre style={errorBlockStyle}>{error}</pre>}
        {!loading && !error && posts.length === 0 && (
          <div style={emptyStyle}>
            <p style={{ margin: 0, fontSize: "1.125rem" }}>No posts yet.</p>
            <p style={{ margin: "0.5rem 0 0", fontSize: 14 }}>
              Open Studio and create one to see it here.
            </p>
          </div>
        )}
        {posts.length > 0 && (
          <ul style={{ listStyle: "none", padding: 0, marginTop: "1.5rem" }}>
            {posts.map((post) => (
              <li key={post.id} style={listItemStyle}>
                <h3 style={{ margin: 0 }}>{post.title}</h3>
                <p style={{ margin: "0.25rem 0 0", color: "#666", fontSize: 14 }}>
                  <code>/{post.slug}</code>
                  {post.publishedAt && ` · ${new Date(post.publishedAt).toLocaleDateString()}`}
                </p>
              </li>
            ))}
          </ul>
        )}
      </main>
    </div>
  );
}

const navStyle: React.CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  alignItems: "center",
  padding: "1rem 2rem",
  background: "white",
  borderBottom: "1px solid #eee",
};
const badgeStyle: React.CSSProperties = {
  marginLeft: 8,
  padding: "2px 6px",
  background: "#fef3c7",
  color: "#92400e",
  borderRadius: 3,
  fontSize: 11,
  fontWeight: 600,
  textTransform: "uppercase",
};
const signOutStyle: React.CSSProperties = {
  background: "transparent",
  border: "1px solid #ddd",
  padding: "0.375rem 0.75rem",
  borderRadius: 4,
  fontSize: 13,
  cursor: "pointer",
};
const errorBlockStyle: React.CSSProperties = {
  background: "#fef2f2",
  padding: "1rem",
  borderRadius: 4,
  color: "#991b1b",
  fontSize: 13,
};
const emptyStyle: React.CSSProperties = {
  background: "white",
  border: "1px dashed #ddd",
  borderRadius: 8,
  padding: "3rem 2rem",
  textAlign: "center",
  color: "#666",
  marginTop: "1.5rem",
};
const listItemStyle: React.CSSProperties = {
  background: "white",
  border: "1px solid #eee",
  borderRadius: 8,
  padding: "1rem 1.25rem",
  marginBottom: "0.5rem",
};
