import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { MANIFEST } from "@/lib/pylon";

export function PoliciesPage() {
	if (MANIFEST.policies.length === 0) {
		return (
			<Card>
				<CardContent className="py-12 text-center text-sm text-muted-foreground">
					No policies defined.
				</CardContent>
			</Card>
		);
	}
	return (
		<Card>
			<CardContent className="p-0">
				<div className="divide-y">
					{MANIFEST.policies.map((p) => (
						<div key={p.name} className="px-4 py-3">
							<div className="flex items-center gap-2">
								<code className="text-sm">{p.name}</code>
								{p.entity && (
									<Badge variant="outline" className="text-[10px]">
										entity: {p.entity}
									</Badge>
								)}
								{p.action && (
									<Badge variant="outline" className="text-[10px]">
										action: {p.action}
									</Badge>
								)}
							</div>
							<p className="mt-1 text-xs text-muted-foreground">
								Policy expressions are masked in the public manifest. Use{" "}
								<code className="rounded bg-muted px-1 py-0.5">
									GET /api/manifest?full=1
								</code>{" "}
								(admin) to view rule bodies.
							</p>
						</div>
					))}
				</div>
			</CardContent>
		</Card>
	);
}
