import React from "react";
import { Link, Image } from "@pylonsync/react";

interface PageProps {
  url: string;
}

const SHOTS = [
  { src: "/sunset.jpg", alt: "Sunset over a ridge", title: "Sunset" },
  { src: "/forest.jpg", alt: "A dense forest canopy", title: "Forest" },
  { src: "/mountain.jpg", alt: "Mountain peak at dawn", title: "Mountain" },
];

export default function GalleryPage({ url }: PageProps) {
  return (
    <div className="space-y-6">
      <section>
        <h1 className="text-3xl font-semibold tracking-tight">
          <code>{`<Image />`}</code>
        </h1>
        <p className="mt-2 text-zinc-600">
          Each tile below is a 2400×1600 JPEG (~250KB) served through Pylon's
          built-in resizer at <code>/_pylon/image</code>. The first request
          resizes + re-encodes (fast_image_resize SIMD + mozjpeg / libwebp).
          Subsequent requests serve the content-addressed cache hit.
        </p>
      </section>

      <ul className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {SHOTS.map((shot) => (
          <li
            key={shot.src}
            className="overflow-hidden rounded-2xl border border-zinc-200 bg-white"
          >
            <Image
              src={shot.src}
              alt={shot.alt}
              width={600}
              height={400}
              quality={70}
              sizes="(max-width: 640px) 100vw, (max-width: 1024px) 50vw, 33vw"
              className="aspect-[3/2] w-full object-cover"
            />
            <div className="flex items-center justify-between p-3">
              <span className="text-sm font-medium">{shot.title}</span>
              <code className="text-xs text-zinc-500">{shot.src}</code>
            </div>
          </li>
        ))}
      </ul>

      <p className="text-xs text-zinc-500">
        Right-click any image and open in a new tab — the URL is the
        optimized variant (smallest <code>w</code> the browser picked for its
        device pixel ratio), and the response is WebP for ~30% smaller bytes
        than the JPEG source.
      </p>

      <p className="text-sm">
        <Link
          href="/"
          className="text-blue-600 underline-offset-4 hover:underline"
        >
          ← back home
        </Link>
      </p>
    </div>
  );
}
