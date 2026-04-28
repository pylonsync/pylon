import Link from "next/link";
import { Button } from "@/components/ui/button";
import { pylon } from "@/lib/pylon";
import { signOut } from "../login/actions";

export default async function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  // Hard auth gate. proxy.ts already redirected unauthenticated users
  // to /login — this is the second line of defense (forged cookies are
  // caught here even if they slipped past the proxy).
  const auth = await pylon.requireAuth();

  return (
    <div>
      <nav className="flex items-center justify-between border-b bg-card px-6 py-4">
        <Link href="/dashboard" className="font-semibold no-underline">
          __APP_NAME__
        </Link>
        <div className="flex items-center gap-3">
          <span className="text-sm text-muted-foreground">
            {auth.userId}
            {auth.isAdmin && (
              <span className="ml-2 rounded bg-yellow-100 px-1.5 py-0.5 text-[10px] font-semibold uppercase text-yellow-900">
                admin
              </span>
            )}
          </span>
          <form action={signOut}>
            <Button type="submit" variant="outline" size="sm">
              Sign out
            </Button>
          </form>
        </div>
      </nav>
      <main className="mx-auto max-w-5xl px-6 py-8">{children}</main>
    </div>
  );
}
