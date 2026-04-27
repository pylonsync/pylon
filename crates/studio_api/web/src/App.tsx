import { useState } from "react";
import { Toaster } from "@/components/ui/sonner";
import { AuthProvider } from "@/auth/AuthContext";
import { StudioLayout, type StudioPage } from "@/layout/StudioLayout";
import { EntitiesPage } from "@/pages/Entities";
import { FunctionsPage } from "@/pages/Functions";
import { HealthPage } from "@/pages/Health";
import { ManifestPage } from "@/pages/Manifest";
import { PoliciesPage } from "@/pages/Policies";
import { RoutesPage } from "@/pages/Routes";
import { SettingsPage } from "@/pages/Settings";
import { SyncPage } from "@/pages/Sync";

export default function App() {
	const [page, setPage] = useState<StudioPage>("entities");
	return (
		<AuthProvider>
			<StudioLayout page={page} onPageChange={setPage}>
				{page === "entities" && <EntitiesPage />}
				{page === "functions" && <FunctionsPage />}
				{page === "manifest" && <ManifestPage />}
				{page === "policies" && <PoliciesPage />}
				{page === "routes" && <RoutesPage />}
				{page === "sync" && <SyncPage />}
				{page === "health" && <HealthPage />}
				{page === "settings" && <SettingsPage />}
			</StudioLayout>
			<Toaster richColors position="bottom-right" />
		</AuthProvider>
	);
}
