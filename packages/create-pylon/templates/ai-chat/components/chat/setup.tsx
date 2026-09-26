"use client";

import React from "react";
import { CodeBlock } from "./markdown";
import { KeyIcon } from "./icons";

const ENV_SNIPPET = `PYLON_LLM_PROVIDER=anthropic
ANTHROPIC_API_KEY=sk-ant-...`;

// Setup steps shown while the server has no model provider key. `compact`
// is the inline version inside a failed reply.
export function ProviderSetup({ compact = false }: { compact?: boolean }) {
  return (
    <div className="bezel">
      <div className={"bezel-core " + (compact ? "p-4" : "p-5 sm:p-6")}>
        <div className="flex items-start gap-3.5">
          <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-warn-soft text-warn">
            <KeyIcon size={18} />
          </span>
          <div className="min-w-0 flex-1">
            <h2 className="text-[15px] font-semibold tracking-[-0.01em] text-ink">
              Add a model provider key
            </h2>
            <p className="mt-1 text-[13.5px] leading-relaxed text-ink-2">
              The server has no API key, so replies are turned off. Messages you send are still saved.
            </p>
          </div>
        </div>

        <ol className="mt-4 space-y-3 text-[13.5px] leading-relaxed text-ink-2">
          <li className="flex gap-3">
            <Step n={1} />
            <div className="min-w-0 flex-1">
              <p>
                Add these lines to <code className="inline-code">.env</code> in the project root.
              </p>
              <div className="mt-2 [&_.code-block]:!mt-0">
                <CodeBlock code={ENV_SNIPPET} lang="env" />
              </div>
            </div>
          </li>
          <li className="flex gap-3">
            <Step n={2} />
            <p className="flex-1">
              Restart <code className="inline-code">pylon dev</code>. The key stays on the server and never
              reaches the browser.
            </p>
          </li>
          {!compact ? (
            <li className="flex gap-3">
              <Step n={3} />
              <p className="flex-1">
                For OpenAI, set <code className="inline-code">PYLON_LLM_PROVIDER=openai</code> and{" "}
                <code className="inline-code">OPENAI_API_KEY</code>, then list OpenAI model ids in{" "}
                <code className="inline-code">lib/site.config.ts</code>.
              </p>
            </li>
          ) : null}
        </ol>
      </div>
    </div>
  );
}

function Step({ n }: { n: number }) {
  return (
    <span className="mt-px flex size-5 shrink-0 items-center justify-center rounded-full bg-hover text-[11px] font-medium tabular-nums text-ink-2">
      {n}
    </span>
  );
}
