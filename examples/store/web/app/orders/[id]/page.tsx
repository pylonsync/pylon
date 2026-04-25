import type { Metadata } from "next";
import OrderClient from "./client";

export const metadata: Metadata = {
  title: "Order",
  description: "Track your order.",
  robots: { index: false, follow: false },
};

export default async function OrderPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  return <OrderClient id={decodeURIComponent(id)} />;
}
