import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { AuthForm } from "../auth-form";

export const metadata: Metadata = {
  title: "Sign in — Acme",
  // Auth pages shouldn't be indexed.
  robots: "noindex",
};

// `app/login/page.tsx` → `/login`. A server-rendered shell around the
// client-side <AuthForm> island.
export default function LoginPage({ auth, response }: PageProps) {
  // Already signed in? Skip the form. `response.redirect` runs in the
  // synchronous shell render, so it's a real 307 before any HTML is sent
  // (no flash, works with JS disabled).
  if (auth.user_id) response.redirect("/dashboard");
  return (
    <div className="mx-auto max-w-sm">
      <Card>
        <CardHeader>
          <CardTitle>Sign in</CardTitle>
          <CardDescription>Welcome back.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <AuthForm mode="login" />
          <p className="text-center text-sm text-muted-foreground">
            No account?{" "}
            <Link
              href="/signup"
              className="font-medium text-foreground hover:underline"
            >
              Create one
            </Link>
          </p>
        </CardContent>
      </Card>
    </div>
  );
}
