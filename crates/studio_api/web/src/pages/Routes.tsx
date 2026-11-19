import { Page, PageScroll } from "@/components/page";
import { Dash } from "@/components/renderers";
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
import { plural } from "@/lib/format";
import { MANIFEST } from "@/lib/pylon";

export function RoutesPage() {
	const routes = MANIFEST.routes;
	return (
		<Page title="Routes" meta={plural(routes.length, "route")}>
			<PageScroll>
				{routes.length === 0 ? (
					<EmptyState
						title="The app has no routes"
						description={
							<>
								Add a page file to the <code>app/</code> directory to declare a
								route.
							</>
						}
					/>
				) : (
					<Table aria-label="Routes">
						<TableHeader>
							<tr>
								<TableHead className="w-80">Path</TableHead>
								<TableHead className="w-28">Mode</TableHead>
								<TableHead className="w-56">Query</TableHead>
								<TableHead>Auth</TableHead>
							</tr>
						</TableHeader>
						<TableBody>
							{routes.map((r, i) => (
								<TableRow key={`${r.path}:${i}`}>
									<TableCell className="font-mono text-xs">{r.path}</TableCell>
									<TableCell>
										<Badge>{r.mode}</Badge>
									</TableCell>
									<TableCell className="font-mono text-xs">
										{r.query ?? <Dash />}
									</TableCell>
									<TableCell>
										{r.auth ? <Badge variant="outline">{r.auth}</Badge> : <Dash />}
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				)}
			</PageScroll>
		</Page>
	);
}
