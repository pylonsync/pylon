import type { OrganizationsConfig } from "./types";

export function orgEntityName(cfg: OrganizationsConfig): string {
	return cfg.entities?.org ?? "Org";
}
export function memberEntityName(cfg: OrganizationsConfig): string {
	return cfg.entities?.member ?? "OrgMember";
}
export function inviteEntityName(cfg: OrganizationsConfig): string {
	return cfg.entities?.invite ?? "OrgInvite";
}
export function teamEntityName(cfg: OrganizationsConfig): string {
	return cfg.entities?.team ?? "Team";
}
export function teamMemberEntityName(cfg: OrganizationsConfig): string {
	return cfg.entities?.teamMember ?? "TeamMember";
}
