import type { Metadata } from "@pylonsync/react";
import { SkillView } from "@pylon-cloud/ui/components/skill-view";

// Two cached shells keyed on the signed-in bit — see app/page.tsx for the
// full auth-bucketed rationale.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

export const metadata: Metadata = {
	title: "Pylon for Claude Code: a drop-in skill",
	description:
		"Copy one file into Claude Code to teach it Pylon's schema, policies, server functions, React client, and deployment workflow.",
	canonical: "/skill",
	openGraph: {
		title: "Pylon for Claude Code: a drop-in skill",
		description:
			"Drop in one file and Claude writes Pylon that compiles, from schema and policies through functions and deployment.",
		url: "https://pylonsync.com/skill",
		type: "article",
	},
	twitter: {
		card: "summary_large_image",
		title: "Pylon for Claude Code",
		description:
			"One file teaches Claude Code how to build Pylon apps correctly.",
	},
};

export default function SkillPage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return <SkillView signedIn={Boolean(session?.exists)} />;
}
