import type { Metadata } from "next";
import AccountClient from "./client";

export const metadata: Metadata = {
  title: "Account",
  description: "Your orders and shipping addresses.",
  robots: { index: false, follow: false },
};

export default function AccountPage() {
  return <AccountClient />;
}
