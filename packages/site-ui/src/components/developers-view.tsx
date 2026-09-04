"use client";

import { Link } from "@pylonsync/react";
import {
	type LucideIcon,
	BookOpen,
	Boxes,
	Braces,
	Bot,
	FileCode2,
	Github,
	History,
	Terminal,
} from "lucide-react";
import { MarketingShell } from "./marketing-shell";

// /developers — one page that names every developer and agent resource Pylon
// publishes, at the URL it lives at.
//
// The point is findability by NAME. "Pylon CLI", "Pylon MCP server", "Pylon
// OpenAPI spec" and "Pylon agent skill" all existed before this page and none
// of them were reachable by searching for what they are called, because no
// page said the words next to the link.

interface Resource {
	name: string;
	href: string;
	external?: boolean;
	what: string;
	how?: string;
	icon: LucideIcon;
}

const BUILD: Resource[] = [
	{
		name: "Pylon documentation",
		href: "https://docs.pylonsync.com",
		external: true,
		what: "Every concept, from entities and policies to auth, plugins, clients, and operations.",
		how: "Add .md to any page URL to read it as markdown.",
		icon: BookOpen,
	},
	{
		name: "Pylon CLI",
		href: "https://www.npmjs.com/package/@pylonsync/cli",
		external: true,
		what: "The official command line tool: dev, build, test, verify, deploy, logs, secrets, domains, and database backups.",
		how: "npm create @pylonsync/pylon@latest my-app — or curl -fsSL https://www.pylonsync.com/install.sh | bash for the global binary. Every command takes --json.",
		icon: Terminal,
	},
	{
		name: "Starter templates",
		href: "/developers/examples",
		what: "Eighteen working apps: SaaS, chat, shop, marketplace, directory, CRM, helpdesk, AI chat, and more.",
		how: "npm create @pylonsync/pylon@latest my-app --template saas",
		icon: Boxes,
	},
	{
		name: "Source on GitHub",
		href: "https://github.com/pylonsync/pylon",
		external: true,
		what: "The Rust runtime, the TypeScript SDKs, the templates, and the example apps. MIT licensed.",
		icon: Github,
	},
	{
		name: "Changelog",
		href: "https://github.com/pylonsync/pylon/releases",
		external: true,
		what: "What shipped, release by release.",
		icon: History,
	},
];

const AGENTS: Resource[] = [
	{
		name: "Pylon agent skill",
		href: "/skill",
		what: "One markdown file that teaches a coding agent to write Pylon that compiles: the schema DSL, the policy language, the function flavors, the hooks, and the footguns.",
		how: "Read it at /pylon-skill.md, or load it into Claude Code as a skill.",
		icon: FileCode2,
	},
	{
		name: "llms.txt",
		href: "/llms.txt",
		what: "What Pylon is for, when to reach for it, and where everything else lives — in the llmstxt.org format an agent reads first.",
		icon: Bot,
	},
	{
		name: "Pylon MCP server",
		href: "/mcp.json",
		what: "A remote Model Context Protocol server over Streamable HTTP. Four read-only tools: search the docs, read a page, list templates, fetch the skill. No account, no key.",
		how: "claude mcp add --transport http pylon https://www.pylonsync.com/mcp",
		icon: Braces,
	},
	{
		name: "Pylon agent API (OpenAPI 3.1)",
		href: "/openapi.json",
		what: "The same four operations as plain HTTP GETs under /agent/v1, with unique operation ids, typed parameters, and response schemas for function calling.",
		how: "curl 'https://www.pylonsync.com/agent/v1/docs/search?q=policies'",
		icon: Braces,
	},
	{
		name: "Markdown pages",
		href: "/index.md",
		what: "Every page on this site is also readable as markdown, with no navigation or scripts around it.",
		how: "Add .md to any path, or send Accept: text/markdown.",
		icon: FileCode2,
	},
];

function ResourceCard({ r }: { r: Resource }) {
	const Icon = r.icon;
	const inner = (
		<>
			<span className="mb-4 flex size-9 items-center justify-center rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper)] text-[var(--color-brand)]">
				<Icon className="size-4" />
			</span>
			<div className="text-[16px] font-semibold tracking-tight text-[var(--color-ink)]">
				{r.name}
			</div>
			<p className="mt-2 flex-1 text-[14px] leading-[1.55] text-[var(--color-ink-3)]">
				{r.what}
			</p>
			{r.how && (
				<code className="mt-4 block overflow-x-auto whitespace-pre rounded-[var(--radius-sm)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-3 py-2 font-mono text-[12px] leading-[1.5] text-[var(--color-ink-2)]">
					{r.how}
				</code>
			)}
		</>
	);
	const cls =
		"group flex flex-col rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] p-6 shadow-[var(--shadow-card)] transition-shadow duration-300 hover:shadow-[var(--shadow-card-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)] focus-visible:ring-offset-2";
	return r.external || r.href.endsWith(".txt") || r.href.endsWith(".json") || r.href.endsWith(".md") ? (
		<a href={r.href} className={cls}>
			{inner}
		</a>
	) : (
		<Link href={r.href} className={cls}>
			{inner}
		</Link>
	);
}

export function DevelopersView({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<MarketingShell signedIn={signedIn}>
			<header className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[1100px] px-5 pb-14 pt-20 sm:px-8 sm:pb-16 sm:pt-28">
					<h1 className="max-w-[20ch] text-[clamp(34px,5vw,56px)] font-semibold leading-[1.03] tracking-[-0.04em] text-[var(--color-ink)]">
						Developer resources
					</h1>
					<p className="mt-6 max-w-[62ch] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Everything Pylon publishes for people writing code, and for the
						agents writing it with them. All of it is public, free, and usable
						without an account.
					</p>
				</div>
			</header>

			<div className="mx-auto max-w-[1100px] px-5 py-16 sm:px-8 sm:py-20">
				<h2 className="text-[22px] font-semibold tracking-tight text-[var(--color-ink)]">
					Build with Pylon
				</h2>
				<div className="mt-6 grid gap-4 sm:grid-cols-2">
					{BUILD.map((r) => (
						<ResourceCard key={r.name} r={r} />
					))}
				</div>

				<h2 className="mt-16 text-[22px] font-semibold tracking-tight text-[var(--color-ink)]">
					For agents
				</h2>
				<p className="mt-3 max-w-[62ch] text-[14.5px] leading-[1.6] text-[var(--color-ink-3)]">
					Pylon is built for coding agents, so its own site answers machines
					directly. Nothing below needs a key, a signup, or a rate-limit
					exemption.
				</p>
				<div className="mt-6 grid gap-4 sm:grid-cols-2">
					{AGENTS.map((r) => (
						<ResourceCard key={r.name} r={r} />
					))}
				</div>

				<section className="mt-16 rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] p-7">
					<h2 className="text-[18px] font-semibold tracking-tight text-[var(--color-ink)]">
						Connect an agent in one command
					</h2>
					<p className="mt-2 max-w-[62ch] text-[14px] leading-[1.6] text-[var(--color-ink-3)]">
						Point Claude Code, or any MCP client, at the hosted server. It
						exposes four read-only tools and needs no credentials.
					</p>
					<code className="mt-4 block overflow-x-auto whitespace-pre rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper)] px-4 py-3 font-mono text-[12.5px] text-[var(--color-ink-2)]">
						claude mcp add --transport http pylon https://www.pylonsync.com/mcp
					</code>
				</section>
			</div>
		</MarketingShell>
	);
}
