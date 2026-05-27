export { SignIn, SignUp } from "./components/SignIn";
export type { SignInProps, SignUpProps } from "./components/SignIn";

export {
	SignedIn,
	SignedOut,
	Protect,
	HasRole,
	InOrg,
	NoOrg,
	RedirectToSignIn,
} from "./components/Gates";
export type {
	ProtectProps,
	HasRoleProps,
	RedirectToSignInProps,
} from "./components/Gates";

export { UserButton } from "./components/UserButton";
export type { UserButtonProps } from "./components/UserButton";

export { SignOutButton } from "./components/SignOutButton";
export type { SignOutButtonProps } from "./components/SignOutButton";

export {
	OrganizationSwitcher,
	CreateOrganization,
} from "./components/OrganizationSwitcher";
export type {
	OrganizationSwitcherProps,
	CreateOrganizationProps,
} from "./components/OrganizationSwitcher";

export { InviteMembers } from "./components/InviteMembers";
export type { InviteMembersProps } from "./components/InviteMembers";

export { useAuth } from "./hooks/useAuth";
export type { UseAuthReturn } from "./hooks/useAuth";

// Re-export the lower-level API helpers so apps building custom auth
// surfaces can drive the same endpoints without duplicating the fetch
// + error-mapping logic the built-in components use.
export {
	ApiError,
	createInvite,
	createOrg,
	listAuthProviders,
	listInvites,
	listOrgMembers,
	listOrgs,
	passwordLogin,
	passwordRegister,
	persistSession,
	removeMember,
	revokeInvite,
	sendMagicLink,
	updateMemberRole,
	verifyMagicLink,
} from "./lib/api";
export type {
	AuthProvider,
	InviteResult,
	OrgMember,
	OrgSummary,
	PendingInvite,
	SessionResponse,
} from "./lib/api";
