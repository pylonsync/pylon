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

export { EnsureGuest } from "./components/EnsureGuest";
export type { EnsureGuestProps } from "./components/EnsureGuest";

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

export { AcceptInvite } from "./components/AcceptInvite";
export type { AcceptInviteProps } from "./components/AcceptInvite";

export { ConnectAccount } from "./components/ConnectAccount";
export type { ConnectAccountProps } from "./components/ConnectAccount";

export { FileUpload } from "./components/FileUpload";
export type {
	FileUploadProps,
	FileUploadResult,
} from "./components/FileUpload";

export { UserProfile } from "./components/UserProfile";
export type { UserProfileProps } from "./components/UserProfile";

export { ForgotPassword, ResetPassword } from "./components/PasswordReset";
export type {
	ForgotPasswordProps,
	ResetPasswordProps,
} from "./components/PasswordReset";

export { ChatBot } from "./components/ChatBot";
export type { ChatBotProps, ChatMessage } from "./components/ChatBot";

export { EntityList } from "./components/EntityList";
export type {
	EntityListProps,
	EntityListColumn,
} from "./components/EntityList";

export { EntityForm } from "./components/EntityForm";
export type {
	EntityFormProps,
	EntityFormField,
} from "./components/EntityForm";

export { Router, Link, Outlet } from "./router/Router";
export type { RouterProps, LinkProps } from "./router/Router";
export type { RouteSpec, MatchedRoute } from "./router/match";
export {
	useRouter,
	useParams,
	useSearchParams,
	usePathname,
} from "./router/useRouter";

export { useAuth } from "./hooks/useAuth";
export type { UseAuthReturn } from "./hooks/useAuth";

// Re-export the lower-level API helpers so apps building custom auth
// surfaces can drive the same endpoints without duplicating the fetch
// + error-mapping logic the built-in components use.
export {
	acceptInvite,
	ApiError,
	changePassword,
	completePasswordReset,
	ensureGuestSession,
	connectionAuthUrl,
	createApiKey,
	createInvite,
	createOrg,
	listActiveSessions,
	listApiKeys,
	listAuthProviders,
	listInvites,
	listOrgMembers,
	listOrgs,
	passwordLogin,
	passwordRegister,
	persistSession,
	removeMember,
	requestPasswordReset,
	revokeAllSessions,
	revokeApiKey,
	revokeInvite,
	sendMagicLink,
	updateMemberRole,
	verifyMagicLink,
} from "./lib/api";
export type {
	ActiveSession,
	ApiKeyCreated,
	ApiKeySummary,
	AuthProvider,
	InviteResult,
	OrgMember,
	OrgSummary,
	PendingInvite,
	SessionResponse,
} from "./lib/api";
