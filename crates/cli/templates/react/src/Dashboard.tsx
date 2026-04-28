import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { pylonJson, type Me } from "@/lib/pylon";

type Post = {
  id: string;
  title: string;
  slug: string;
  body?: string;
  publishedAt?: string | null;
};

export function Dashboard({ me, onSignOut }: { me: Me; onSignOut: () => void }) {
  const [posts, setPosts] = useState<Post[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    pylonJson<Post[]>("/api/entities/Post")
      .then((data) => {
        setPosts(data);
        setError(null);
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, []);

  return (
    <div>
      <nav className="flex items-center justify-between border-b bg-card px-6 py-4">
        <strong>__APP_NAME__</strong>
        <div className="flex items-center gap-3">
          <span className="text-sm text-muted-foreground">
            {me.user_id}
            {me.is_admin && (
              <span className="ml-2 rounded bg-yellow-100 px-1.5 py-0.5 text-[10px] font-semibold uppercase text-yellow-900">
                admin
              </span>
            )}
          </span>
          <Button onClick={onSignOut} variant="outline" size="sm">
            Sign out
          </Button>
        </div>
      </nav>
      <main className="mx-auto max-w-5xl space-y-6 px-6 py-8">
        <header>
          <h1 className="text-3xl font-semibold tracking-tight">Dashboard</h1>
          <p className="mt-1 text-muted-foreground">
            Posts loaded from <code className="font-mono text-sm">/api/entities/Post</code>. Add
            some via the API or{" "}
            <a
              href="http://localhost:4321/studio"
              target="_blank"
              rel="noreferrer"
              className="text-primary underline-offset-4 hover:underline"
            >
              Studio
            </a>
            .
          </p>
        </header>

        {loading && <p className="text-muted-foreground">Loading…</p>}
        {error && (
          <pre className="rounded bg-destructive/10 p-4 text-sm text-destructive">{error}</pre>
        )}
        {!loading && !error && posts.length === 0 && (
          <Card className="border-dashed">
            <CardContent className="py-12 text-center text-muted-foreground">
              <p className="text-lg">No posts yet.</p>
              <p className="mt-1 text-sm">Open Studio and create one to see it here.</p>
            </CardContent>
          </Card>
        )}
        {posts.length > 0 && (
          <ul className="grid gap-2">
            {posts.map((post) => (
              <li key={post.id}>
                <Card>
                  <CardHeader className="py-4">
                    <CardTitle className="text-base">{post.title}</CardTitle>
                    <CardDescription>
                      <code className="font-mono text-xs">/{post.slug}</code>
                      {post.publishedAt && ` · ${new Date(post.publishedAt).toLocaleDateString()}`}
                    </CardDescription>
                  </CardHeader>
                </Card>
              </li>
            ))}
          </ul>
        )}
      </main>
    </div>
  );
}
