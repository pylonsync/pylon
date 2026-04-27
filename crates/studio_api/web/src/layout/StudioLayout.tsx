import { useState } from "react";
import {
	Activity,
	Box,
	Database,
	FileCode,
	FileText,
	LogIn,
	LogOut,
	Lock,
	Radio,
	Settings,
	ShieldCheck,
	Zap,
} from "lucide-react";
import {
	Sidebar,
	SidebarContent,
	SidebarFooter,
	SidebarGroup,
	SidebarGroupContent,
	SidebarGroupLabel,
	SidebarHeader,
	SidebarInset,
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	SidebarProvider,
	SidebarTrigger,
} from "@/components/ui/sidebar";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useAuth } from "@/auth/AuthContext";
import { SignInDialog } from "@/auth/SignInDialog";
import { MANIFEST } from "@/lib/pylon";

export type StudioPage =
	| "entities"
	| "functions"
	| "manifest"
	| "policies"
	| "routes"
	| "sync"
	| "health"
	| "settings";

type NavItem = {
	id: StudioPage;
	label: string;
	icon: React.ComponentType<{ className?: string }>;
	requiresAdmin?: boolean;
};

const NAV: { section: string; items: NavItem[] }[] = [
	{
		section: "Data",
		items: [
			{ id: "entities", label: "Entities", icon: Database },
			{ id: "manifest", label: "Manifest", icon: FileText },
		],
	},
	{
		section: "Logic",
		items: [
			{ id: "functions", label: "Functions", icon: FileCode, requiresAdmin: true },
			{ id: "policies", label: "Policies", icon: ShieldCheck },
			{ id: "routes", label: "Routes", icon: Box },
		],
	},
	{
		section: "Operations",
		items: [
			{ id: "sync", label: "Live sync", icon: Radio },
			{ id: "health", label: "Health", icon: Activity, requiresAdmin: true },
		],
	},
];

export function StudioLayout({
	page,
	onPageChange,
	children,
}: {
	page: StudioPage;
	onPageChange: (next: StudioPage) => void;
	children: React.ReactNode;
}) {
	const { me, hasToken, signOut } = useAuth();
	const [signInOpen, setSignInOpen] = useState(false);
	const isAdmin = !!me?.is_admin;

	return (
		<SidebarProvider>
			<Sidebar variant="inset">
				<SidebarHeader>
					<div className="flex items-center gap-2 px-2 py-1.5">
						<div className="flex size-8 items-center justify-center rounded-md bg-primary text-primary-foreground">
							<Zap className="size-4" />
						</div>
						<div className="flex flex-col leading-tight">
							<span className="text-sm font-semibold">Pylon Studio</span>
							<span className="text-xs text-muted-foreground">
								{MANIFEST.name} · v{MANIFEST.version}
							</span>
						</div>
					</div>
				</SidebarHeader>
				<SidebarContent>
					{NAV.map((group) => (
						<SidebarGroup key={group.section}>
							<SidebarGroupLabel>{group.section}</SidebarGroupLabel>
							<SidebarGroupContent>
								<SidebarMenu>
									{group.items.map((item) => {
										const locked = item.requiresAdmin && !isAdmin;
										const Icon = item.icon;
										return (
											<SidebarMenuItem key={item.id}>
												<SidebarMenuButton
													isActive={page === item.id}
													onClick={() => onPageChange(item.id)}
													tooltip={
														locked ? `${item.label} — admin required` : item.label
													}
												>
													<Icon />
													<span>{item.label}</span>
													{locked && (
														<Lock className="ml-auto size-3 opacity-60" />
													)}
												</SidebarMenuButton>
											</SidebarMenuItem>
										);
									})}
								</SidebarMenu>
							</SidebarGroupContent>
						</SidebarGroup>
					))}
				</SidebarContent>
				<SidebarFooter>
					<SidebarMenu>
						<SidebarMenuItem>
							{hasToken ? (
								<DropdownMenu>
									<DropdownMenuTrigger asChild>
										<SidebarMenuButton tooltip="Account">
											<div className="flex size-6 items-center justify-center rounded-full bg-primary text-xs font-semibold text-primary-foreground">
												{isAdmin ? "A" : me?.user_id?.slice(0, 1).toUpperCase() ?? "U"}
											</div>
											<div className="flex flex-col items-start leading-tight">
												<span className="text-xs font-medium">
													{isAdmin ? "Admin" : me?.user_id ?? "Signed in"}
												</span>
												<span className="text-[10px] text-muted-foreground">
													{isAdmin ? "Full access" : "Limited"}
												</span>
											</div>
										</SidebarMenuButton>
									</DropdownMenuTrigger>
									<DropdownMenuContent side="right" align="end" className="min-w-44">
										<DropdownMenuLabel className="text-xs font-normal">
											{me?.user_id ?? "anonymous"}
										</DropdownMenuLabel>
										<DropdownMenuSeparator />
										<DropdownMenuItem onClick={() => onPageChange("settings")}>
											<Settings className="size-4" />
											Settings
										</DropdownMenuItem>
										<DropdownMenuItem onClick={signOut}>
											<LogOut className="size-4" />
											Sign out
										</DropdownMenuItem>
									</DropdownMenuContent>
								</DropdownMenu>
							) : (
								<SidebarMenuButton onClick={() => setSignInOpen(true)}>
									<LogIn />
									<span>Sign in</span>
								</SidebarMenuButton>
							)}
						</SidebarMenuItem>
					</SidebarMenu>
				</SidebarFooter>
			</Sidebar>
			<SidebarInset>
				<header className="flex h-14 shrink-0 items-center gap-2 border-b px-4">
					<SidebarTrigger className="-ml-1" />
					<Separator orientation="vertical" className="mr-2 h-4" />
					<h1 className="text-sm font-medium capitalize">{page}</h1>
					<div className="ml-auto flex items-center gap-2">
						{!hasToken && (
							<Button
								size="sm"
								variant="outline"
								onClick={() => setSignInOpen(true)}
							>
								<LogIn className="size-3.5" /> Sign in
							</Button>
						)}
						{hasToken && (
							<Badge variant={isAdmin ? "default" : "secondary"}>
								{isAdmin ? "Admin" : "Signed in"}
							</Badge>
						)}
					</div>
				</header>
				<div className="p-6">{children}</div>
			</SidebarInset>
			<SignInDialog open={signInOpen} onOpenChange={setSignInOpen} />
		</SidebarProvider>
	);
}
