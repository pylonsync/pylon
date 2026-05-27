"use client";

import {
	type ChangeEvent,
	type DragEvent,
	type ReactNode,
	useCallback,
	useRef,
	useState,
} from "react";
import { uploadFile, type UploadedFile } from "@pylonsync/react";
import { cn } from "../lib/cn";

export interface FileUploadProps {
	/** Called once for each successfully uploaded file. */
	onUploaded?: (file: UploadedFile, source: File) => void;
	/** Called once with the full batch after all files finish (errors and all). */
	onComplete?: (results: FileUploadResult[]) => void;
	/** MIME / extension hint passed to `<input accept=...>`. */
	accept?: string;
	/** Allow multi-file selection / drop. Default: false. */
	multiple?: boolean;
	/** Max file size in bytes — files exceeding this are rejected client-side. */
	maxSizeBytes?: number;
	/** Override the headline. */
	label?: ReactNode;
	/** Override the helper text. */
	helperText?: ReactNode;
	className?: string;
}

export interface FileUploadResult {
	file: File;
	status: "ok" | "error";
	uploaded?: UploadedFile;
	error?: string;
}

/**
 * Drop-in drag-and-drop file picker. Streams each file to
 * `/api/files/upload` via the existing `uploadFile` helper, surfaces
 * per-file progress + errors, and emits `onUploaded` as each file
 * lands.
 *
 * Pure UI shell — no provider configuration of its own. Pylon's file
 * router handles where bytes actually go (local disk for self-host,
 * Stack0 / S3 for cloud).
 */
export function FileUpload({
	onUploaded,
	onComplete,
	accept,
	multiple,
	maxSizeBytes,
	label = "Drop files to upload",
	helperText = "or click to browse",
	className,
}: FileUploadProps) {
	const [dragging, setDragging] = useState(false);
	const [items, setItems] = useState<FileUploadResult[]>([]);
	const inputRef = useRef<HTMLInputElement | null>(null);

	const startUpload = useCallback(
		async (files: FileList | File[]) => {
			const arr = Array.from(files);
			if (arr.length === 0) return;
			const initial: FileUploadResult[] = arr.map((file) => {
				if (maxSizeBytes && file.size > maxSizeBytes) {
					return {
						file,
						status: "error",
						error: `Larger than ${humanSize(maxSizeBytes)}`,
					};
				}
				return { file, status: "ok" };
			});
			setItems((prev) => [...prev, ...initial]);

			const settled = await Promise.all(
				initial.map(async (entry) => {
					if (entry.status === "error") return entry;
					try {
						const uploaded = await uploadFile(entry.file);
						onUploaded?.(uploaded, entry.file);
						return { ...entry, uploaded } satisfies FileUploadResult;
					} catch (err) {
						return {
							...entry,
							status: "error" as const,
							error: err instanceof Error ? err.message : "Upload failed",
						};
					}
				}),
			);
			setItems((prev) => {
				const next = prev.slice();
				for (const result of settled) {
					const idx = next.findIndex(
						(r) => r.file === result.file && r !== result,
					);
					if (idx >= 0) next[idx] = result;
				}
				return next;
			});
			onComplete?.(settled);
		},
		[maxSizeBytes, onComplete, onUploaded],
	);

	function onDrop(e: DragEvent<HTMLDivElement>) {
		e.preventDefault();
		setDragging(false);
		if (e.dataTransfer?.files) void startUpload(e.dataTransfer.files);
	}

	function onChange(e: ChangeEvent<HTMLInputElement>) {
		if (e.target.files) void startUpload(e.target.files);
		// Reset the input so re-selecting the same file fires `change`
		// again — the browser dedupes by value otherwise.
		e.target.value = "";
	}

	return (
		<div className={cn("space-y-3", className)}>
			<div
				role="button"
				tabIndex={0}
				onClick={() => inputRef.current?.click()}
				onKeyDown={(e) => {
					if (e.key === "Enter" || e.key === " ") inputRef.current?.click();
				}}
				onDragOver={(e) => {
					e.preventDefault();
					setDragging(true);
				}}
				onDragLeave={() => setDragging(false)}
				onDrop={onDrop}
				className={cn(
					"flex cursor-pointer flex-col items-center justify-center gap-1 rounded-xl border border-dashed px-6 py-10 text-center transition-colors",
					dragging
						? "border-[var(--pylon-ink,#0a0a0a)] bg-[var(--pylon-paper-2,#f4f4f5)]"
						: "border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] hover:bg-[var(--pylon-paper-2,#f4f4f5)]",
				)}
			>
				<p className="text-sm font-medium text-[var(--pylon-ink,#0a0a0a)]">
					{label}
				</p>
				<p className="text-xs text-[var(--pylon-ink-3,#71717a)]">
					{helperText}
				</p>
				<input
					ref={inputRef}
					type="file"
					accept={accept}
					multiple={multiple}
					onChange={onChange}
					className="sr-only"
				/>
			</div>
			{items.length > 0 ? (
				<ul className="divide-y divide-[var(--pylon-rule,#e5e7eb)] rounded-md border border-[var(--pylon-rule,#e5e7eb)]">
					{items.map((item, i) => (
						<li
							key={`${item.file.name}-${i}`}
							className="flex items-center justify-between gap-3 px-3 py-2 text-sm"
						>
							<span className="min-w-0">
								<span className="block truncate font-medium text-[var(--pylon-ink,#0a0a0a)]">
									{item.file.name}
								</span>
								<span className="block text-[11px] text-[var(--pylon-ink-3,#71717a)]">
									{humanSize(item.file.size)}
									{item.error ? ` · ${item.error}` : ""}
								</span>
							</span>
							<UploadStatus result={item} />
						</li>
					))}
				</ul>
			) : null}
		</div>
	);
}

function UploadStatus({ result }: { result: FileUploadResult }) {
	if (result.uploaded) {
		return (
			<span className="rounded-md bg-[var(--pylon-paper-2,#f4f4f5)] px-2 py-0.5 text-[11px] font-medium text-[var(--pylon-ink-2,#52525b)]">
				Uploaded
			</span>
		);
	}
	if (result.status === "error") {
		return (
			<span className="rounded-md bg-[var(--pylon-error-bg,#fef2f2)] px-2 py-0.5 text-[11px] font-medium text-[var(--pylon-error-ink,#b91c1c)]">
				Failed
			</span>
		);
	}
	return (
		<span className="text-[11px] text-[var(--pylon-ink-3,#71717a)]">
			Uploading…
		</span>
	);
}

function humanSize(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
