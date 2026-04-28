import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { LoginForm } from "./form";

export default function LoginPage({
  searchParams,
}: {
  searchParams: { next?: string };
}) {
  return (
    <main className="mx-auto max-w-md px-6 py-16">
      <Card>
        <CardHeader>
          <CardTitle>Sign in</CardTitle>
          <CardDescription>We'll email you a 6-digit code.</CardDescription>
        </CardHeader>
        <CardContent>
          <LoginForm next={searchParams.next} />
          <p className="mt-6 text-xs text-muted-foreground">
            In dev mode the code is also printed to the Pylon server's stdout, and appears below the
            form once requested.
          </p>
        </CardContent>
      </Card>
    </main>
  );
}
