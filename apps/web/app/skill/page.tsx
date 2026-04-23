import type { Metadata } from "next";
import fs from "node:fs";
import path from "node:path";
import SkillClient from "./skill-client";

export const metadata: Metadata = {
  title: "Pylon for Claude Code — drop-in skill",
  description:
    "Copy a single skill file into your Claude Code setup and Claude will know how to build Pylon apps correctly — schema, policies, functions, React client, deployment.",
};

export default function SkillPage() {
  const skill = fs.readFileSync(
    path.join(process.cwd(), "public", "pylon-skill.md"),
    "utf8",
  );

  return (
    <main className="skill-page">
      <div className="skill-shell">
        <a href="/" className="skill-back">
          ← Pylon
        </a>

        <h1 className="skill-title">
          Claude Code <span className="skill-title-accent">skill</span>
        </h1>
        <p className="skill-lede">
          Drop this file into your Claude Code setup and Claude will know how
          to build Pylon apps correctly — schema, policies, server functions,
          React client, deployment. Updates to the skill ship with Pylon.
        </p>

        <div className="skill-install">
          <h2 className="skill-h2">Install</h2>
          <ol className="skill-steps">
            <li>
              Copy the skill below.
            </li>
            <li>
              Save it to{" "}
              <code>~/.claude/skills/pylon/SKILL.md</code> (user-wide) or{" "}
              <code>.claude/skills/pylon/SKILL.md</code> in your repo
              (project-scoped).
            </li>
            <li>
              Restart Claude Code. Claude now loads the skill whenever you work
              on a Pylon project or ask to build one.
            </li>
          </ol>
          <p className="skill-hint">
            Prefer a one-liner?{" "}
            <code>
              mkdir -p ~/.claude/skills/pylon && curl -fsSL
              https://pylonsync.com/pylon-skill.md &gt; ~/.claude/skills/pylon/SKILL.md
            </code>
          </p>
        </div>

        <SkillClient content={skill} />
      </div>
    </main>
  );
}
