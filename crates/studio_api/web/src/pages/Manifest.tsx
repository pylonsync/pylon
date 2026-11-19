import { CodeBlock } from "@/components/code-block";
import { Page } from "@/components/page";
import { Dash } from "@/components/renderers";
import { EmptyState } from "@/components/states";
import { Stat, StatStrip } from "@/components/stat-strip";
import { Badge } from "@/components/ui/badge";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { Tabs, TabsList, TabsPanel, TabsTab } from "@/components/ui/tabs";
import { onLinkClick } from "@/lib/navigation";
import { MANIFEST, type ManifestField } from "@/lib/pylon";
import { routeToPath } from "@/lib/route";

export function ManifestPage() {
	return (
		<Page title="Manifest">
			<StatStrip>
				<Stat label="Entities" value={MANIFEST.entities.length} />
				<Stat label="Queries" value={MANIFEST.queries.length} />
				<Stat label="Actions" value={MANIFEST.actions.length} />
				<Stat label="Routes" value={MANIFEST.routes.length} />
			</StatStrip>
			<Tabs defaultValue="entities" className="flex-1">
				<TabsList aria-label="Manifest parts">
					<TabsTab value="entities">Entities</TabsTab>
					<TabsTab value="queries">Queries</TabsTab>
					<TabsTab value="actions">Actions</TabsTab>
					<TabsTab value="raw">JSON</TabsTab>
				</TabsList>
				<TabsPanel value="entities" className="overflow-auto">
					{MANIFEST.entities.length === 0 ? (
						<EmptyState
							title="The app has no entities"
							description={
								<>
									Declare an entity with <code>entity()</code> in{" "}
									<code>app.ts</code>.
								</>
							}
						/>
					) : (
						<Table aria-label="Entity fields">
							<TableHeader>
								<tr>
									<TableHead className="w-64">Entity</TableHead>
									<TableHead className="w-64">Field</TableHead>
									<TableHead className="w-64">Type</TableHead>
									<TableHead>Flags</TableHead>
								</tr>
							</TableHeader>
							<TableBody>
								{MANIFEST.entities.flatMap((e) => {
									const url = routeToPath({ kind: "resource", entity: e.name });
									return e.fields.map((f, i) => (
										<TableRow key={`${e.name}.${f.name}`}>
											<TableCell className={i > 0 ? "text-transparent" : "font-medium"}>
												{i === 0 ? (
													<span className="inline-flex items-center gap-2">
														<a
															href={url}
															className="text-link underline-offset-2 hover:underline"
															onClick={(ev) => onLinkClick(ev, url)}
														>
															{e.name}
														</a>
														{e.crdt && <Badge variant="blue">CRDT</Badge>}
													</span>
												) : (
													<span aria-hidden="true">{e.name}</span>
												)}
											</TableCell>
											<TableCell className="font-mono text-xs">{f.name}</TableCell>
											<TableCell className="font-mono text-xs text-muted-foreground">
												{f.type}
											</TableCell>
											<TableCell>
												<span className="inline-flex gap-1">
													{f.optional && <Badge variant="outline">optional</Badge>}
													{f.unique && <Badge variant="outline">unique</Badge>}
													{!f.optional && !f.unique && <Dash />}
												</span>
											</TableCell>
										</TableRow>
									));
								})}
							</TableBody>
						</Table>
					)}
				</TabsPanel>
				<TabsPanel value="queries" className="overflow-auto">
					<SignatureTable
						label="Queries"
						items={MANIFEST.queries}
						empty="The app has no custom queries"
					/>
				</TabsPanel>
				<TabsPanel value="actions" className="overflow-auto">
					<SignatureTable
						label="Actions"
						items={MANIFEST.actions}
						empty="The app has no custom actions"
					/>
				</TabsPanel>
				<TabsPanel value="raw" className="overflow-auto p-3">
					<CodeBlock value={JSON.stringify(MANIFEST, null, 2)} />
				</TabsPanel>
			</Tabs>
		</Page>
	);
}

function SignatureTable({
	label,
	items,
	empty,
}: {
	label: string;
	items: { name: string; input: ManifestField[] }[];
	empty: string;
}) {
	if (items.length === 0) return <EmptyState title={empty} />;
	return (
		<Table aria-label={label}>
			<TableHeader>
				<tr>
					<TableHead className="w-80">Name</TableHead>
					<TableHead>Input</TableHead>
				</tr>
			</TableHeader>
			<TableBody>
				{items.map((item) => (
					<TableRow key={item.name}>
						<TableCell className="font-mono text-xs">{item.name}</TableCell>
						<TableCell className="max-w-none font-mono text-xs text-muted-foreground">
							{item.input.length > 0 ? (
								item.input
									.map((f) => `${f.name}${f.optional ? "?" : ""}: ${f.type}`)
									.join(", ")
							) : (
								<Dash />
							)}
						</TableCell>
					</TableRow>
				))}
			</TableBody>
		</Table>
	);
}
