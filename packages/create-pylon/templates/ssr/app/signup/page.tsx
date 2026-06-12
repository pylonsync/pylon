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
  title: "Create your account — __APP_NAME__",
  robots: "noindex",
};

// `app/signup/page.tsx` → `/signup`. Same shell as /login, register mode.
export default function SignupPage({ auth, response }: PageProps) {
  if (auth.user_id) response.redirect("/dashboard");
  return (
    <div className="mx-auto max-w-sm">
      <Card>
        <CardHeader>
          <CardTitle>Create your account</CardTitle>
          <CardDescription>
            Email + password. No credit card, no email verification step in dev.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <AuthForm mode="signup" />
          <p className="text-center text-sm text-muted-foreground">
            Already have an account?{" "}
            <Link
              href="/login"
              className="font-medium text-foreground hover:underline"
            >
              Sign in
            </Link>
          </p>
        </CardContent>
      </Card>
    </div>
  );
}
