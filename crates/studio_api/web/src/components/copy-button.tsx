import { useEffect, useState } from "react";
import { Check, Copy } from "lucide-react";

import { toast } from "@/lib/toast";
import { copyToClipboard } from "@/lib/row-action";
import { cn } from "@/lib/utils";

/**
 * An icon button that copies `value` to the clipboard. The icon is a check
 * for a moment after the copy.
 */
export function CopyButton({
	value,
	label,
	className,
}: {
	value: string;
	/** The accessible name, such as "Copy the id". */
	label: string;
	className?: string;
}) {
	const [copied, setCopied] = useState(false);
	useEffect(() => {
		if (!copied) return;
		const t = window.setTimeout(() => setCopied(false), 1200);
		return () => window.clearTimeout(t);
	}, [copied]);

	return (
		<button
			type="button"
			aria-label={label}
			title={label}
			className={cn(
				"inline-flex size-5 shrink-0 cursor-default items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground",
				className,
			)}
			onClick={async (e) => {
				e.stopPropagation();
				if (await copyToClipboard(value)) setCopied(true);
				else toast.error("The clipboard is not available", { description: value });
			}}
		>
			{copied ? <Check className="size-3" /> : <Copy className="size-3" />}
		</button>
	);
}
