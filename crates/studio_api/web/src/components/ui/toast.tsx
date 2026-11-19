import {
	Toast,
	ToastClose,
	ToastDescription,
	ToastRegion,
	ToastTitle,
} from "@ui-sdk/react/toast";
import { X } from "lucide-react";

/**
 * The place where toasts show. Render it one time, near the root of the app.
 * Call `toast.success()`, `toast.error()` or `toast.info()` from `@/lib/toast`
 * to show one.
 */
function Toaster() {
	return (
		<ToastRegion side="bottom" align="end" maxVisible={4}>
			{() => (
				<Toast className="group/toast rounded-md border border-border bg-popover text-popover-foreground shadow-[0_8px_24px_-6px_rgb(0_0_0/0.2)]">
					<div className="flex items-start gap-2.5 py-2.5 pr-9 pl-3">
						<span
							aria-hidden="true"
							className="mt-[5px] size-2 shrink-0 rounded-full bg-status-gray-fg group-data-[type=error]/toast:bg-status-red-fg group-data-[type=info]/toast:bg-status-blue-fg group-data-[type=success]/toast:bg-status-green-fg group-data-[type=warning]/toast:bg-status-amber-fg"
						/>
						<div className="min-w-0 flex-1">
							<ToastTitle className="text-[13px] font-medium break-words" />
							<ToastDescription className="mt-0.5 text-xs break-words text-muted-foreground" />
						</div>
					</div>
					<ToastClose
						aria-label="Dismiss"
						className="absolute top-2 right-2 flex size-5 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
					>
						<X className="size-3" />
					</ToastClose>
				</Toast>
			)}
		</ToastRegion>
	);
}

export { Toaster };
