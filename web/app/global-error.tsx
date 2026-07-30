"use client";

// Last-resort error boundary: catches errors thrown inside the root
// layout itself (where error.tsx can't reach, because error.tsx is a
// CHILD of the layout it's protecting). Next requires this file to
// render its own <html> + <body> since the root layout is gone.
//
// In practice this fires almost never — only if our root layout's
// providers / metadata generation explode. The bare HTML keeps the
// "white screen of nothing" failure mode from happening.
export default function GlobalError({
	error,
	reset,
}: {
	error: Error & { digest?: string };
	reset: () => void;
}) {
	return (
		<html lang="en">
			<body
				style={{
					margin: 0,
					minHeight: "100vh",
					display: "flex",
					alignItems: "center",
					justifyContent: "center",
					fontFamily:
						"-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
					background: "#fff",
					color: "#0a0a0a",
				}}
			>
				<div style={{ maxWidth: 420, padding: "0 1.5rem", textAlign: "center" }}>
					<h1 style={{ fontSize: 18, fontWeight: 600, margin: 0 }}>
						Pylon Cloud crashed
					</h1>
					<p
						style={{
							fontSize: 13,
							color: "#6b7280",
							marginTop: 8,
							marginBottom: 16,
						}}
					>
						{error.message || "An unexpected error occurred."}
					</p>
					<button
						type="button"
						onClick={reset}
						style={{
							background: "#0a0a0a",
							color: "#fff",
							border: 0,
							borderRadius: 8,
							padding: "8px 16px",
							fontSize: 13,
							cursor: "pointer",
						}}
					>
						Try again
					</button>
					{error.digest && (
						<div
							style={{
								marginTop: 16,
								fontSize: 11,
								fontFamily: "ui-monospace, monospace",
								color: "#9ca3af",
							}}
						>
							{error.digest}
						</div>
					)}
				</div>
			</body>
		</html>
	);
}
