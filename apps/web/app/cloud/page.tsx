import type { Metadata } from "next";
import { CloudPage } from "./cloud-page";

export const metadata: Metadata = {
  title: "Pylon Cloud — managed Pylon, escape hatch included",
  description:
    "Deploy your Pylon backend in one command. Free tier, global edge, same binary you run locally. Pay when you outgrow it — or take the binary and self-host.",
  openGraph: {
    title: "Pylon Cloud — managed Pylon, escape hatch included",
    description:
      "Deploy in one command. Same binary as local. Free tier, global edge, scale to your AWS account whenever you want.",
    url: "https://pylonsync.com/cloud",
    siteName: "Pylon",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "Pylon Cloud — managed Pylon",
    description:
      "Deploy in one command. Same binary as local. Free tier — pay only when you outgrow it.",
  },
};

export default function Cloud() {
  return <CloudPage />;
}
