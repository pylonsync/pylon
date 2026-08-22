// What an agent can actually do with pylonsync.com, in one place.
//
// The same four operations are exposed twice — as MCP tools over
// `POST /mcp`, and as REST endpoints under `/agent/v1` described by
// `/openapi.json`. Two transports, one implementation: an agent that speaks
// MCP and a script that speaks HTTP must not get different answers, and a
// second copy is how they would.

import { docsIndex, rankDocs, readDoc, type DocEntry } from "./docs-index";
import {
  GROUPS,
  LIVE_DEMOS,
  createCommand,
  templateRepoUrl,
  type Example,
} from "@pylon-cloud/ui/lib/examples-content";
import { DOCS_URL, GITHUB_URL, SITE_URL } from "../site";

export interface DocHit {
  title: string;
  url: string;
  /** The markdown source of the same page — what to read next. */
  markdownUrl: string;
  path: string;
  summary?: string;
}

export interface TemplateInfo {
  /** The `--template` value. */
  template: string;
  name: string;
  blurb: string;
  /** Pylon features the template exercises. */
  shows: string[];
  /** The exact command that scaffolds it. */
  command: string;
  /** Source on GitHub. */
  source: string;
  /** A deployed demo, when one exists. */
  demo?: string;
}

function toHit(entry: DocEntry): DocHit {
  return {
    title: entry.title,
    url: entry.url,
    markdownUrl: `${entry.url}.md`,
    path: entry.path,
    summary: entry.summary,
  };
}

/** Search the Pylon documentation index. */
export async function searchDocs(query: string, limit = 10): Promise<DocHit[]> {
  const entries = await docsIndex();
  const capped = Math.min(Math.max(Math.trunc(limit) || 10, 1), 50);
  return rankDocs(entries, query, capped).map(toHit);
}

/** The whole documentation index, unranked. */
export async function listDocs(): Promise<DocHit[]> {
  return (await docsIndex()).map(toHit);
}

/** One documentation page, as markdown. */
export async function fetchDoc(path: string) {
  return readDoc(path);
}

/**
 * Every create-pylon template the site documents, with the command that
 * scaffolds it. Derived from the same content the /developers/examples page
 * renders, so the list can't drift from what a visitor sees.
 */
export function listTemplates(): TemplateInfo[] {
  const all: Example[] = [...LIVE_DEMOS, ...GROUPS.flatMap((g) => g.items)];
  const seen = new Set<string>();
  const out: TemplateInfo[] = [];
  for (const example of all) {
    if (!example.template || seen.has(example.template)) continue;
    seen.add(example.template);
    out.push({
      template: example.template,
      name: example.name,
      blurb: example.blurb,
      shows: example.shows,
      command: createCommand(example.template),
      source: templateRepoUrl(example.template),
      demo: example.live,
    });
  }
  return out.sort((a, b) => a.template.localeCompare(b.template));
}

/**
 * The Pylon agent skill: the single file that teaches a coding agent to write
 * Pylon that compiles. Served from this app's own `public/` copy, which the
 * image build refreshes from the framework repo — so the tool answers with the
 * same bytes as `https://www.pylonsync.com/pylon-skill.md`.
 */
export async function readSkill(): Promise<{ url: string; markdown: string }> {
  const url = `${SITE_URL}/pylon-skill.md`;
  const file = Bun.file("public/pylon-skill.md");
  if (await file.exists()) {
    return { url, markdown: await file.text() };
  }
  // The file ships in the image; a miss means someone changed the build. Say
  // so plainly rather than returning an empty document that reads as "the
  // skill is empty".
  return {
    url,
    markdown: `# Pylon skill\n\nThe skill file is not on disk in this deployment. Read it at ${url} or at ${GITHUB_URL}/blob/main/skills/pylon/SKILL.md.\n`,
  };
}

/** Where an agent should look when it wants more than these tools give. */
export const AGENT_RESOURCES = {
  docs: DOCS_URL,
  llmsTxt: `${SITE_URL}/llms.txt`,
  openapi: `${SITE_URL}/openapi.json`,
  mcp: `${SITE_URL}/mcp`,
  skill: `${SITE_URL}/pylon-skill.md`,
  install: `${SITE_URL}/install.sh`,
  github: GITHUB_URL,
} as const;
