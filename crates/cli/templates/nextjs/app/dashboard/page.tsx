import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { pylon } from "@/lib/pylon";

type Post = {
  id: string;
  title: string;
  slug: string;
  body?: string;
  publishedAt?: string | null;
};

export default async function DashboardPage() {
  // Server-side fetch via @pylonsync/next — automatically forwards the
  // session cookie and throws ApiError on non-2xx.
  const posts = await pylon.json<Post[]>("/api/entities/Post").catch(() => []);

  return (
    <div className="space-y-6">
      <header>
        <h1 className="text-3xl font-semibold tracking-tight">Dashboard</h1>
        <p className="mt-1 text-muted-foreground">
          Server-rendered list, fetched via <code className="font-mono text-sm">pylon.json()</code>. Add
          posts via the API or{" "}
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

      {posts.length === 0 ? (
        <Card className="border-dashed">
          <CardContent className="py-12 text-center text-muted-foreground">
            <p className="text-lg">No posts yet.</p>
            <p className="mt-1 text-sm">Open Studio and create one to see it here.</p>
          </CardContent>
        </Card>
      ) : (
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
    </div>
  );
}
