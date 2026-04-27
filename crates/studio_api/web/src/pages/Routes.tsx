import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { MANIFEST } from "@/lib/pylon";

export function RoutesPage() {
	if (MANIFEST.routes.length === 0) {
		return (
			<Card>
				<CardContent className="py-12 text-center text-sm text-muted-foreground">
					No custom routes defined.
				</CardContent>
			</Card>
		);
	}
	return (
		<Card>
			<CardContent className="p-0">
				<div className="divide-y">
					{MANIFEST.routes.map((r) => (
						<div key={r.path} className="flex items-center gap-2 px-4 py-3">
							<code className="text-sm">{r.path}</code>
							<Badge variant="secondary">{r.mode}</Badge>
							{r.query && (
								<span className="text-xs text-muted-foreground">
									query={r.query}
								</span>
							)}
							{r.auth && (
								<Badge variant="outline" className="text-[10px]">
									auth: {r.auth}
								</Badge>
							)}
						</div>
					))}
				</div>
			</CardContent>
		</Card>
	);
}
