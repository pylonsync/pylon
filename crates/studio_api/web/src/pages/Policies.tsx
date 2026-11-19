import { Page, PageScroll } from "@/components/page";
import { EmptyState } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { Dash } from "@/components/renderers";
import { plural } from "@/lib/format";
import { MANIFEST } from "@/lib/pylon";

export function PoliciesPage() {
	const policies = MANIFEST.policies;
	return (
		<Page title="Policies" meta={plural(policies.length, "policy", "policies")}>
			<PageScroll>
				{policies.length === 0 ? (
					<EmptyState
						title="The app has no policies"
						description={
							<>
								An entity with no policy denies every request. Add policies
								with <code>policy()</code> in <code>app.ts</code>.
							</>
						}
					/>
				) : (
					<>
						<Table aria-label="Policies">
							<TableHeader>
								<tr>
									<TableHead className="w-80">Name</TableHead>
									<TableHead className="w-48">Entity</TableHead>
									<TableHead>Action</TableHead>
								</tr>
							</TableHeader>
							<TableBody>
								{policies.map((p, i) => (
									<TableRow key={`${p.name}:${i}`}>
										<TableCell className="font-mono text-xs">{p.name}</TableCell>
										<TableCell>
											{p.entity ? <Badge variant="outline">{p.entity}</Badge> : <Dash />}
										</TableCell>
										<TableCell>
											{p.action ? <Badge variant="outline">{p.action}</Badge> : <Dash />}
										</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Table>
						<p className="px-3 py-2.5 text-xs text-muted-foreground">
							Studio does not show policy rules. To read them, request{" "}
							<code>GET /api/manifest?full=1</code> as an admin.
						</p>
					</>
				)}
			</PageScroll>
		</Page>
	);
}
