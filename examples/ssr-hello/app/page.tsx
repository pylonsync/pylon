import React from "react";
import { Link, Image } from "@pylonsync/react";

interface PageProps {
  url: string;
}

export default function IndexPage({ url }: PageProps) {
  return (
    <div className="space-y-8">
      <section>
        <h1 className="text-3xl font-semibold tracking-tight">
          Pylon SSR — Phase 2
        </h1>
        <p className="mt-2 text-zinc-600">
          File-based routes, per-route chunk splitting, instant client-side
          navigation, and built-in image optimization. One binary, one port.
        </p>
      </section>

      <section className="rounded-2xl border border-zinc-200 bg-white p-6">
        <h2 className="text-lg font-semibold">Try it</h2>
        <ul className="mt-3 space-y-2 text-sm">
          <li>
            <Link
              href="/hello"
              className="text-blue-600 underline-offset-4 hover:underline"
            >
              /hello
            </Link>{" "}
            — interactive counter (proves hydration).
          </li>
          <li>
            <Link
              href="/hello?name=eric"
              className="text-blue-600 underline-offset-4 hover:underline"
            >
              /hello?name=eric
            </Link>{" "}
            — search params flow through SSR.
          </li>
          <li>
            <Link
              href="/gallery"
              className="text-blue-600 underline-offset-4 hover:underline"
            >
              /gallery
            </Link>{" "}
            — <code>&lt;Image&gt;</code> demo with the built-in resizer.
          </li>
        </ul>
        <p className="mt-4 text-xs text-zinc-500">
          Click any link: the browser does <em>not</em> reload — Pylon fetches
          the new route's SSR HTML, dynamic-imports its chunk (already
          preloaded if the link was in viewport), and re-renders into the
          same React root.
        </p>
      </section>

      <p className="text-xs text-zinc-500">
        You're at <code>{url}</code>. View source: the HTML carries{" "}
        <code>{`<link rel="modulepreload">`}</code> for the shared chunk and
        a per-route <code>{`<script type="module">`}</code>.
      </p>
    </div>
  );
}
