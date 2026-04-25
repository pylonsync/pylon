import Link from "next/link";
import { Button } from "@pylonsync/example-ui/button";

export default function NotFound() {
  return (
    <main className="mx-auto flex max-w-md flex-col items-center gap-4 p-16 text-center">
      <h1 className="text-3xl font-semibold">Not found</h1>
      <p className="text-sm text-muted-foreground">
        The page or product you&rsquo;re looking for doesn&rsquo;t exist.
      </p>
      <Button asChild>
        <Link href="/">Back to catalog</Link>
      </Button>
    </main>
  );
}
