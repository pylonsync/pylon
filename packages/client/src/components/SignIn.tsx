"use client";

import {
	type FormEvent,
	type ReactNode,
	useEffect,
	useState,
} from "react";
import {
	ApiError,
	type AuthProvider,
	listAuthProviders,
	passwordLogin,
	persistSession,
	sendMagicLink,
	verifyMagicLink,
} from "../lib/api";
import { cn } from "../lib/cn";

export interface SignInProps {
	/** Auth method to lead with. Default: "magic" (most secure default). */
	method?: "magic" | "password";
	/** Where the dashboard sends the user after sign-in. */
	afterSignInUrl?: string;
	/** Optional callback once a session is minted. */
	onSignedIn?: () => void;
	/** Forgot-password landing route shown when the password tab is active. */
	forgotPasswordUrl?: string;
	/** Text shown above the form. */
	title?: ReactNode;
	/** Subtitle / call-to-action. */
	subtitle?: ReactNode;
	/** Tailwind class merged onto the card. */
	className?: string;
}

export function SignIn({
	method = "magic",
	afterSignInUrl,
	onSignedIn,
	forgotPasswordUrl = "/forgot-password",
	title = "Sign in",
	subtitle,
	className,
}: SignInProps) {
	const [tab, setTab] = useState<"magic" | "password">(method);

	return (
		<Card className={className}>
			<Heading title={title} subtitle={subtitle} />
			<TabBar value={tab} onChange={setTab} />
			{tab === "magic" ? (
				<MagicLinkPanel
					afterSignInUrl={afterSignInUrl}
					onSignedIn={onSignedIn}
				/>
			) : (
				<PasswordPanel
					mode="login"
					afterSignInUrl={afterSignInUrl}
					onSignedIn={onSignedIn}
					forgotPasswordUrl={forgotPasswordUrl}
				/>
			)}
			<OAuthButtons />
			<Switcher
				prompt="No account?"
				cta="Create one"
				href={afterSignInUrl ? `?signup=1` : "#signup"}
			/>
		</Card>
	);
}

export interface SignUpProps extends SignInProps {}

export function SignUp({
	afterSignInUrl,
	onSignedIn,
	title = "Create your account",
	subtitle,
	className,
}: SignUpProps) {
	return (
		<Card className={className}>
			<Heading title={title} subtitle={subtitle} />
			<PasswordPanel
				mode="register"
				afterSignInUrl={afterSignInUrl}
				onSignedIn={onSignedIn}
			/>
			<OAuthButtons />
			<Switcher
				prompt="Already have an account?"
				cta="Sign in"
				href={afterSignInUrl ? `?signin=1` : "#signin"}
			/>
		</Card>
	);
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

function Card({ className, children }: { className?: string; children: ReactNode }) {
	return (
		<div
			className={cn(
				"pylon-auth-card",
				"mx-auto w-full max-w-sm space-y-5 rounded-xl border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] p-7 shadow-sm",
				className,
			)}
		>
			{children}
		</div>
	);
}

function Heading({ title, subtitle }: { title: ReactNode; subtitle?: ReactNode }) {
	return (
		<div className="space-y-1.5 text-center">
			<h2 className="text-lg font-semibold tracking-tight text-[var(--pylon-ink,#0a0a0a)]">
				{title}
			</h2>
			{subtitle ? (
				<p className="text-sm text-[var(--pylon-ink-2,#52525b)]">{subtitle}</p>
			) : null}
		</div>
	);
}

function TabBar({
	value,
	onChange,
}: {
	value: "magic" | "password";
	onChange: (v: "magic" | "password") => void;
}) {
	return (
		<div className="flex rounded-md border border-[var(--pylon-rule,#e5e7eb)] p-0.5 text-sm">
			<TabButton active={value === "magic"} onClick={() => onChange("magic")}>
				Magic link
			</TabButton>
			<TabButton
				active={value === "password"}
				onClick={() => onChange("password")}
			>
				Password
			</TabButton>
		</div>
	);
}

function TabButton({
	active,
	onClick,
	children,
}: {
	active: boolean;
	onClick: () => void;
	children: ReactNode;
}) {
	return (
		<button
			type="button"
			onClick={onClick}
			className={cn(
				"flex-1 rounded-[5px] py-1.5 text-center font-medium transition-colors",
				active
					? "bg-[var(--pylon-ink,#0a0a0a)] text-[var(--pylon-paper,#ffffff)]"
					: "text-[var(--pylon-ink-2,#52525b)] hover:text-[var(--pylon-ink,#0a0a0a)]",
			)}
		>
			{children}
		</button>
	);
}

function MagicLinkPanel({
	afterSignInUrl,
	onSignedIn,
}: {
	afterSignInUrl?: string;
	onSignedIn?: () => void;
}) {
	const [email, setEmail] = useState("");
	const [code, setCode] = useState("");
	const [step, setStep] = useState<"email" | "code">("email");
	const [error, setError] = useState<string | null>(null);
	const [pending, setPending] = useState(false);

	async function onRequest(e: FormEvent) {
		e.preventDefault();
		setError(null);
		setPending(true);
		try {
			await sendMagicLink(email);
			setStep("code");
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	async function onVerify(e: FormEvent) {
		e.preventDefault();
		setError(null);
		setPending(true);
		try {
			const session = await verifyMagicLink(email, code);
			persistSession(session);
			onSignedIn?.();
			if (afterSignInUrl && typeof window !== "undefined") {
				window.location.assign(afterSignInUrl);
			}
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	if (step === "email") {
		return (
			<form onSubmit={onRequest} className="space-y-3">
				<Field
					label="Email"
					type="email"
					value={email}
					onChange={setEmail}
					required
					autoComplete="email"
					placeholder="you@example.com"
				/>
				<SubmitButton pending={pending} label="Send code" />
				<ErrorText message={error} />
			</form>
		);
	}

	return (
		<form onSubmit={onVerify} className="space-y-3">
			<p className="text-center text-sm text-[var(--pylon-ink-2,#52525b)]">
				We sent a code to <span className="font-medium">{email}</span>.
			</p>
			<Field
				label="Code"
				value={code}
				onChange={setCode}
				required
				autoComplete="one-time-code"
				inputMode="numeric"
				placeholder="123456"
			/>
			<SubmitButton pending={pending} label="Sign in" />
			<ErrorText message={error} />
			<button
				type="button"
				onClick={() => {
					setStep("email");
					setCode("");
					setError(null);
				}}
				className="block w-full text-center text-xs text-[var(--pylon-ink-3,#71717a)] hover:underline"
			>
				Use a different email
			</button>
		</form>
	);
}

function PasswordPanel({
	mode,
	afterSignInUrl,
	onSignedIn,
	forgotPasswordUrl,
}: {
	mode: "login" | "register";
	afterSignInUrl?: string;
	onSignedIn?: () => void;
	forgotPasswordUrl?: string;
}) {
	const [email, setEmail] = useState("");
	const [password, setPassword] = useState("");
	const [displayName, setDisplayName] = useState("");
	const [error, setError] = useState<string | null>(null);
	const [pending, setPending] = useState(false);

	async function onSubmit(e: FormEvent) {
		e.preventDefault();
		setError(null);
		setPending(true);
		try {
			const session =
				mode === "login"
					? await passwordLogin({ email, password })
					: await (
							await import("../lib/api")
						).passwordRegister({
							email,
							password,
							displayName: displayName || undefined,
						});
			persistSession(session);
			onSignedIn?.();
			if (afterSignInUrl && typeof window !== "undefined") {
				window.location.assign(afterSignInUrl);
			}
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	return (
		<form onSubmit={onSubmit} className="space-y-3">
			{mode === "register" ? (
				<Field
					label="Name"
					value={displayName}
					onChange={setDisplayName}
					autoComplete="name"
					placeholder="optional"
				/>
			) : null}
			<Field
				label="Email"
				type="email"
				value={email}
				onChange={setEmail}
				required
				autoComplete="email"
				placeholder="you@example.com"
			/>
			<Field
				label="Password"
				type="password"
				value={password}
				onChange={setPassword}
				required
				autoComplete={mode === "login" ? "current-password" : "new-password"}
			/>
			<SubmitButton
				pending={pending}
				label={mode === "login" ? "Sign in" : "Create account"}
			/>
			{mode === "login" && forgotPasswordUrl ? (
				<a
					href={forgotPasswordUrl}
					className="block text-center text-xs text-[var(--pylon-ink-3,#71717a)] hover:underline"
				>
					Forgot password?
				</a>
			) : null}
			<ErrorText message={error} />
		</form>
	);
}

function OAuthButtons() {
	const [providers, setProviders] = useState<AuthProvider[] | null>(null);
	useEffect(() => {
		let cancelled = false;
		void listAuthProviders().then((p) => {
			if (!cancelled) setProviders(p);
		});
		return () => {
			cancelled = true;
		};
	}, []);
	if (!providers || providers.length === 0) return null;
	return (
		<div className="space-y-2">
			<Divider label="or" />
			{providers.map((p) => (
				<button
					key={p.provider}
					type="button"
					onClick={() => {
						if (typeof window !== "undefined") {
							const callback = encodeURIComponent(window.location.href);
							window.location.assign(`${p.auth_url}?callback=${callback}`);
						}
					}}
					className="flex w-full items-center justify-center gap-2 rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-3 py-2 text-sm font-medium text-[var(--pylon-ink,#0a0a0a)] transition-colors hover:bg-[var(--pylon-paper-2,#f4f4f5)]"
				>
					Continue with {labelFor(p.provider)}
				</button>
			))}
		</div>
	);
}

function Switcher({
	prompt,
	cta,
	href,
}: {
	prompt: string;
	cta: string;
	href: string;
}) {
	return (
		<p className="text-center text-xs text-[var(--pylon-ink-2,#52525b)]">
			{prompt}{" "}
			<a
				href={href}
				className="font-medium text-[var(--pylon-ink,#0a0a0a)] hover:underline"
			>
				{cta}
			</a>
		</p>
	);
}

function Field({
	label,
	value,
	onChange,
	type = "text",
	required,
	autoComplete,
	placeholder,
	inputMode,
}: {
	label: string;
	value: string;
	onChange: (v: string) => void;
	type?: string;
	required?: boolean;
	autoComplete?: string;
	placeholder?: string;
	inputMode?: "text" | "numeric";
}) {
	return (
		<label className="block space-y-1.5">
			<span className="text-xs font-medium text-[var(--pylon-ink-2,#52525b)]">
				{label}
			</span>
			<input
				type={type}
				value={value}
				onChange={(e) => onChange(e.target.value)}
				required={required}
				autoComplete={autoComplete}
				placeholder={placeholder}
				inputMode={inputMode}
				className="w-full rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-3 py-2 text-sm text-[var(--pylon-ink,#0a0a0a)] placeholder:text-[var(--pylon-ink-3,#a1a1aa)] focus:border-[var(--pylon-ink,#0a0a0a)] focus:outline-none"
			/>
		</label>
	);
}

function SubmitButton({ pending, label }: { pending: boolean; label: string }) {
	return (
		<button
			type="submit"
			disabled={pending}
			className="w-full rounded-md bg-[var(--pylon-ink,#0a0a0a)] px-3 py-2 text-sm font-medium text-[var(--pylon-paper,#ffffff)] transition-opacity hover:opacity-90 disabled:opacity-60"
		>
			{pending ? "…" : label}
		</button>
	);
}

function ErrorText({ message }: { message: string | null }) {
	if (!message) return null;
	return (
		<p className="rounded-md border border-[var(--pylon-error-rule,#fecaca)] bg-[var(--pylon-error-bg,#fef2f2)] px-3 py-2 text-xs text-[var(--pylon-error-ink,#b91c1c)]">
			{message}
		</p>
	);
}

function Divider({ label }: { label: string }) {
	return (
		<div className="flex items-center gap-3 text-[10px] uppercase tracking-wider text-[var(--pylon-ink-3,#a1a1aa)]">
			<span className="h-px flex-1 bg-[var(--pylon-rule,#e5e7eb)]" />
			{label}
			<span className="h-px flex-1 bg-[var(--pylon-rule,#e5e7eb)]" />
		</div>
	);
}

function messageFromError(err: unknown): string {
	if (err instanceof ApiError) {
		switch (err.code) {
			case "INVALID_CREDENTIALS":
				return "Wrong email or password.";
			case "USER_EXISTS":
				return "That email is already in use.";
			case "WEAK_PASSWORD":
				return "Pick a stronger password.";
			case "RATE_LIMITED":
				return "Too many attempts — try again in a minute.";
			case "CAPTCHA_REQUIRED":
				return "Captcha verification failed.";
			case "INVALID_CODE":
				return "That code didn't match. Try again.";
			default:
				return err.message;
		}
	}
	if (err instanceof Error) return err.message;
	return "Something went wrong.";
}

function labelFor(provider: string): string {
	if (!provider) return "provider";
	return provider.charAt(0).toUpperCase() + provider.slice(1);
}
