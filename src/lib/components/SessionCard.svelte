<script lang="ts">
	import type { Session } from '$lib/types';
	import { SessionStatus } from '$lib/types';
	import { marked } from 'marked';
	import DOMPurify from 'dompurify';

	interface Props {
		session: Session;
		onexpand?: () => void;
		onstop?: () => void;
	}

	let { session, onexpand, onstop }: Props = $props();

	// Extract project directory name from projectPath
	let projectName = $derived(
		(() => {
			const parts = session.projectPath.split(/[/\\]/).filter(p => p);
			return parts.pop() || session.projectPath;
		})()
	);

	// Parse git status string (e.g., "+1 -0") and return formatted display
	function parseGitStatus(status: string | null): { added: number; deleted: number; hasChanges: boolean } | null {
		if (!status) return null;
		const match = status.match(/\+(\d+)\s+-(\d+)/);
		if (!match) return null;
		const added = parseInt(match[1], 10);
		const deleted = parseInt(match[2], 10);
		return {
			added,
			deleted,
			hasChanges: added > 0 || deleted > 0
		};
	}

	let gitStatusInfo = $derived(parseGitStatus(session.gitStatus));

	// Format context usage (e.g., "136K/200K (68%)")
	function formatContextUsage(usage: { used: number; max: number; percentage: number } | null): string {
		if (!usage) return '';
		const formatTokens = (n: number) => {
			if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
			if (n >= 1_000) return `${(n / 1_000).toFixed(0)}K`;
			return n.toString();
		};
		return `${formatTokens(usage.used)}/${formatTokens(usage.max)} (${usage.percentage.toFixed(0)}%)`;
	}

	let needsAttention = $derived(
		session.status === SessionStatus.NeedsAttention ||
			session.status === SessionStatus.WaitingForInput
	);

	let isPermission = $derived(session.status === SessionStatus.NeedsAttention);
	let isWaitingInput = $derived(session.status === SessionStatus.WaitingForInput);
	let isWorking = $derived(session.status === SessionStatus.Working);
	let isConnecting = $derived(session.status === SessionStatus.Connecting);

	let tooltipText = $state('');
	let tooltipX = $state(0);
	let tooltipY = $state(0);

	function tipEnter(text: string) { tooltipText = text; }
	function tipLeave() { tooltipText = ''; }
	function tipMove(e: MouseEvent) { tooltipX = e.clientX + 12; tooltipY = e.clientY + 12; }

	let cardTitle = $derived(session.customTitle || session.summary || session.firstPrompt);

let renderedUserMessage = $derived.by(() => {
	if (!session.latestUserMessage) return '';
	const rawHtml = marked.parse(session.latestUserMessage, { async: false, breaks: true, gfm: true });
	return DOMPurify.sanitize(rawHtml as string);
});

let renderedAssistantMessage = $derived.by(() => {
	if (!session.latestMessage) return '';
	const rawHtml = marked.parse(session.latestMessage, { async: false, breaks: true, gfm: true });
	return DOMPurify.sanitize(rawHtml as string);
});

	function getStatusLabel(): string {
		switch (session.status) {
			case SessionStatus.Working:
				return 'Working';
			case SessionStatus.NeedsAttention:
				if (session.pendingToolName === 'Question' || session.pendingToolName === 'AskUserQuestion') {
					return 'Waiting for Response';
				}
				return 'Approval Required';
			case SessionStatus.WaitingForInput:
				return 'Ready';
			case SessionStatus.Connecting:
				return 'Connecting';
			default:
				return 'Unknown';
		}
	}

	function formatTimeSince(isoTimestamp: string): string {
		const now = new Date().getTime();
		const then = new Date(isoTimestamp).getTime();
		const diffMs = now - then;
		const diffMins = Math.floor(diffMs / 60000);
		const diffHours = Math.floor(diffMs / 3600000);
		const diffDays = Math.floor(diffMs / 86400000);

		if (diffMins < 1) return 'now';
		if (diffMins < 60) return `${diffMins}m`;
		if (diffHours < 24) return `${diffHours}h`;
		return `${diffDays}d`;
	}

	function handleCardClick(e: MouseEvent) {
		const target = e.target as HTMLElement;
		if (target.closest('.close-btn')) {
			return;
		}
		onexpand?.();
	}

	function handleCardKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' || e.key === ' ') {
			const target = e.target as HTMLElement;
			if (target.classList.contains('session-card')) {
				e.preventDefault();
				onexpand?.();
			}
		}
	}

	let showConfirm = $state(false);

	function handleStop(e: MouseEvent) {
		e.stopPropagation();
		showConfirm = true;
	}

	function confirmStop() {
		showConfirm = false;
		onstop?.();
	}

	function cancelStop(e: MouseEvent) {
		e.stopPropagation();
		showConfirm = false;
	}

	let idCopied = $state(false);

	async function copySessionId(e: MouseEvent) {
		e.stopPropagation();
		try {
			await navigator.clipboard.writeText(session.id);
			idCopied = true;
			tooltipText = 'Copied!';
			setTimeout(() => { idCopied = false; tooltipText = ''; }, 1500);
		} catch { /* clipboard API may fail in some contexts */ }
	}

</script>

<div
	class="session-card"
	class:attention={needsAttention}
	class:permission={isPermission}
	class:waiting={isWaitingInput}
	class:working={isWorking}
	class:connecting={isConnecting}
	onclick={handleCardClick}
	onkeydown={handleCardKeydown}
	role="button"
	tabindex="0"
>
	<!-- Status Header Bar -->
	<div class="status-header-bar">
		<div class="status-header-bar-left">
			<span class="status-indicator"></span>
			<span class="status-label">{getStatusLabel()}</span>
		</div>
		<div class="status-header-bar-right">
			<span class="project-name-badge">{projectName}</span>
			<button type="button" class="close-btn" onclick={handleStop} title="Stop session">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<line x1="18" y1="6" x2="6" y2="18" />
					<line x1="6" y1="6" x2="18" y2="18" />
				</svg>
			</button>
		</div>
	</div>

	<!-- Card Content -->
	<div class="card-body">
		<!-- Header Row: Title (left 50%) + Context Usage (right 50%) -->
		<div class="card-header-row">
			<div class="card-header-left">
				<span class="session-icon">
					<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
						<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
					</svg>
				</span>
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<h3
					class="card-main-title"
					onmouseenter={() => tipEnter(session.id)}
					onmouseleave={tipLeave}
					onmousemove={tipMove}
				>
					{cardTitle}
				</h3>
				<!-- svelte-ignore a11y_click_events_have_key_events -->
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<span
					class="copy-id-btn"
					class:copied={idCopied}
					onclick={copySessionId}
					onmouseenter={() => tipEnter('Copy session ID')}
					onmouseleave={tipLeave}
					onmousemove={tipMove}
				>
					{#if idCopied}
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12" /></svg>
					{:else}
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2" ry="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></svg>
					{/if}
				</span>
			</div>
			<!-- Context Usage - Right 50% -->
			{#if session.contextUsage}
				<div class="context-usage-mini" title="Context window usage: {formatContextUsage(session.contextUsage)}">
					<div class="context-progress-mini" style="width: {session.contextUsage.percentage}%"></div>
					<span class="context-text-mini">{formatContextUsage(session.contextUsage)}</span>
				</div>
			{/if}
		</div>

		<!-- Info Row: Git (left) + Stats (right) -->
		<div class="info-row">
			<!-- Git Info - Left Aligned -->
			{#if session.gitBranch}
				<div class="git-info">
					<div class="git-branch">
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
							<line x1="6" y1="3" x2="6" y2="15" />
							<circle cx="18" cy="6" r="3" />
							<circle cx="6" cy="18" r="3" />
							<path d="M18 9a9 9 0 0 1-9 9" />
						</svg>
						<span class="branch-name">{session.gitBranch}</span>
					</div>
					<span class="git-status" class:has-changes={gitStatusInfo && gitStatusInfo.hasChanges}>
						(+{gitStatusInfo ? gitStatusInfo.added : 0},-{gitStatusInfo ? gitStatusInfo.deleted : 0})
					</span>
				</div>
			{/if}
			<!-- Stats - Right Aligned -->
			<div class="header-stats">
				<span class="message-count">
					<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
						<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
					</svg>
					{session.messageCount}
				</span>
				<span class="time-badge">
					<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
						<circle cx="12" cy="12" r="10" />
						<polyline points="12 6 12 12 16 14" />
					</svg>
					{formatTimeSince(session.modified)}
				</span>
			</div>
		</div>

		<!-- Message Preview -->
		<div class="task-preview">
			{#if renderedUserMessage}
				<div class="message-row user">
					<span class="message-label">You:</span>
					<span class="message-content">{@html renderedUserMessage}</span>
				</div>
			{/if}
			{#if renderedAssistantMessage}
				<div class="message-row assistant">
					<span class="message-label">Claude:</span>
					<span class="message-content">{@html renderedAssistantMessage}</span>
				</div>
			{/if}
		</div>
	</div>

	<!-- Confirmation Dialog -->
	{#if showConfirm}
		<div class="confirm-overlay" onclick={cancelStop}></div>
		<div class="confirm-dialog">
			<p class="confirm-message">Stop this session?</p>
			<div class="confirm-actions">
				<button type="button" class="confirm-btn cancel" onclick={cancelStop}>Cancel</button>
				<button type="button" class="confirm-btn danger" onclick={confirmStop}>Stop</button>
			</div>
		</div>
	{/if}
</div>

<style>
	.session-card {
		position: relative;
		display: flex;
		flex-direction: column;
		background: var(--bg-card);
		border: 1px solid var(--border-default);
		border-radius: var(--radius-lg);
		cursor: pointer;
		transition: all 0.2s cubic-bezier(0.16, 1, 0.3, 1);
		text-align: left;
		width: 100%;
		height: 352px; /* 235px * 1.5 = 352.5px */
		overflow: hidden;
	}

	.session-card:hover {
		border-color: var(--text-muted);
		box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
		background: var(--bg-card-hover);
	}

	/* Card Body */
	.card-body {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: var(--space-sm);
		padding: var(--space-md) var(--space-lg) var(--space-lg);
	}

	/* Header Row: Title (left 50%) + Context Usage (right 50%) */
	.card-header-row {
		display: flex;
		align-items: center;
		gap: var(--space-md);
		width: 100%;
	}

	.card-header-left {
		flex: 1;
		display: flex;
		align-items: center;
		gap: var(--space-sm);
		min-width: 0;
		max-width: 50%;
	}

	.card-header-left .card-main-title {
		font-size: 18px;
		-webkit-line-clamp: 1;
		line-clamp: 1;
	}

	/* Context Usage Mini - Right 50% */
	.context-usage-mini {
		flex: 1;
		position: relative;
		width: 100%;
		height: 24px;
		background: var(--bg-base);
		border: 1px solid var(--border-default);
		border-radius: var(--radius-md);
		overflow: hidden;
	}

	.context-progress-mini {
		position: absolute;
		left: 0;
		top: 0;
		bottom: 0;
		background: linear-gradient(90deg, var(--status-working), var(--status-input));
		opacity: 0.7;
		transition: width 0.3s ease;
	}

	.context-text-mini {
		position: absolute;
		left: 50%;
		top: 50%;
		transform: translate(-50%, -50%);
		font-family: var(--font-mono);
		font-size: 13px;
		font-weight: 600;
		color: var(--text-primary);
		text-shadow: 0 1px 3px rgba(0, 0, 0, 0.8);
		white-space: nowrap;
		letter-spacing: 0.05em;
		z-index: 1;
	}

	.header-stats {
		margin-left: auto;
		display: flex;
		align-items: center;
		gap: var(--space-md);
	}

	.session-icon {
		display: flex;
		align-items: center;
		justify-content: center;
		flex-shrink: 0;
		color: var(--text-muted);
	}

	.card-main-title {
		font-family: var(--font-sans);
		font-size: 22.5px; /* 15px * 1.5 = 22.5px */
		font-weight: 600;
		color: var(--text-primary);
		margin: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		letter-spacing: 0.05em;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		cursor: default;
	}

	.copy-id-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		flex-shrink: 0;
		width: 20px;
		height: 20px;
		color: var(--text-muted);
		cursor: pointer;
		opacity: 0;
		transition: opacity var(--transition-fast), color var(--transition-fast);
	}

	.copy-id-btn:hover {
		opacity: 1 !important;
		color: var(--text-primary);
	}

	.copy-id-btn.copied {
		opacity: 1 !important;
		color: var(--status-input);
	}

	.git-branch {
		display: flex;
		align-items: center;
		gap: 6px;
		font-family: var(--font-mono);
		font-size: 15px;
		color: var(--text-secondary);
		min-width: 0;
	}

	.git-branch svg {
		flex-shrink: 0;
	}

	.branch-name {
		overflow: hidden;
		white-space: nowrap;
		text-overflow: ellipsis;
		min-width: 0;
		max-width: 200px;
	}

	.git-status {
		font-family: var(--font-mono);
		font-size: 15px;
		font-weight: 500;
		color: var(--text-muted);
		white-space: nowrap;
	}

	.git-status.has-changes {
		color: var(--status-permission);
	}

	.message-count {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-family: var(--font-mono);
		font-size: 15px;
		color: var(--text-muted);
	}

	.time-badge {
		font-family: var(--font-mono);
		font-size: 15px;
		font-weight: 500;
		color: var(--text-muted);
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}

	/* Task Preview */
	.task-preview {
		font-size: 14px;
		color: var(--text-secondary);
		line-height: 1.6;
		margin-top: var(--space-xs);
		margin-bottom: 0;
		flex: 1;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: var(--space-xs);
	}

	.message-row {
		display: flex;
		gap: var(--space-sm);
		align-items: flex-start;
	}

	.message-row.user .message-label {
		color: var(--status-input);
		font-weight: 600;
		flex-shrink: 0;
	}

	.message-row.assistant .message-label {
		color: var(--status-working);
		font-weight: 600;
		flex-shrink: 0;
	}

	.message-content {
		flex: 1;
		min-width: 0;
	}

	.message-content :global(p) {
		margin: 0 0 var(--space-xs) 0;
	}

	.message-content :global(p:last-child) {
		margin-bottom: 0;
	}



	.task-preview :global(code) {
		background: var(--bg-base);
		padding: 2px 6px;
		border-radius: 4px;
		font-family: var(--font-mono);
		font-size: 13px;
		color: var(--text-primary);
	}

	.task-preview :global(pre) {
		background: var(--bg-base);
		padding: var(--space-sm);
		border-radius: var(--radius-md);
		overflow-x: auto;
		margin: var(--space-xs) 0;
		white-space: pre-wrap;
		word-wrap: break-word;
	}

	.task-preview :global(pre code) {
		background: transparent;
		padding: 0;
	}

	.task-preview :global(strong) {
		color: var(--text-primary);
		font-weight: 600;
	}

	.task-preview :global(a) {
		color: var(--text-primary);
		text-decoration: underline;
	}

	.task-preview :global(ul),
	.task-preview :global(ol) {
		margin: var(--space-xs) 0;
		padding-left: var(--space-md);
	}

	.task-preview :global(li) {
		margin-bottom: var(--space-xs);
	}



	.task-preview :global(code) {
		background: var(--bg-base);
		padding: 2px 6px;
		border-radius: 4px;
		font-family: var(--font-mono);
		font-size: 13px;
		color: var(--text-primary);
	}

	.task-preview :global(pre) {
		background: var(--bg-base);
		padding: var(--space-sm);
		border-radius: var(--radius-md);
		overflow-x: auto;
		margin: var(--space-xs) 0;
	}

	.task-preview :global(pre code) {
		background: transparent;
		padding: 0;
	}

	.task-preview :global(strong) {
		color: var(--text-primary);
		font-weight: 600;
	}

	.task-preview :global(a) {
		color: var(--text-primary);
		text-decoration: underline;
	}

	/* Status Header Bar - matches MONITOR page status-header style */
	.status-header-bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--space-sm);
		padding: var(--space-sm) var(--space-md);
		background: rgba(255, 255, 255, 0.03);
		border-left: 3px solid var(--border-default);
		border-radius: var(--radius-lg) var(--radius-lg) 0 0;
	}

	.status-header-bar-right {
		display: flex;
		align-items: center;
		gap: 2px;
	}

	.status-header-bar-left {
		display: flex;
		align-items: center;
		gap: var(--space-sm);
	}

	.close-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		padding: 0;
		background: transparent;
		border: 1px solid transparent;
		border-radius: 4px;
		color: var(--text-muted);
		cursor: pointer;
		transition: all 0.2s ease;
	}

	.close-btn:hover {
		background: var(--status-permission);
		color: var(--bg-base);
	}

	.status-header-bar .status-indicator {
		width: 9px; /* 6px * 1.5 = 9px */
		height: 9px; /* 6px * 1.5 = 9px */
		border-radius: 50%;
		background: var(--text-muted);
	}

	.status-header-bar .status-label {
		font-family: var(--font-mono);
		font-size: 18px; /* 12px * 1.5 = 18px */
		font-weight: 500;
		text-transform: uppercase;
		letter-spacing: 0.1em;
		color: var(--text-secondary);
	}

	.project-name-badge {
		font-family: var(--font-mono);
		font-size: 16px;
		font-weight: 600;
		color: var(--text-primary);
		text-align: right;
	}

	/* Info Row - Git (left) + Stats (right) */
	.info-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--space-md);
		margin-top: 0;
		padding-top: var(--space-xs);
		border-top: 1px solid var(--border-default);
	}

	.git-info {
		display: flex;
		align-items: center;
		gap: var(--space-md);
	}

	/* Status-specific colors for header bar - matches MONITOR page */
	.session-card.attention .status-header-bar {
		border-left-color: var(--status-permission);
	}
	.session-card.attention .status-header-bar .status-label {
		color: var(--status-permission);
	}

	.session-card.permission .status-header-bar {
		border-left-color: var(--status-permission);
	}
	.session-card.permission .status-header-bar .status-label {
		color: var(--status-permission);
	}

	.session-card.waiting .status-header-bar {
		border-left-color: var(--status-input);
	}
	.session-card.waiting .status-header-bar .status-label {
		color: var(--status-input);
	}

	.session-card.working .status-header-bar {
		border-left-color: var(--status-working);
	}
	.session-card.working .status-header-bar .status-label {
		color: var(--status-working);
	}

	.session-card.connecting .status-header-bar {
		border-left-color: var(--status-connecting);
	}
	.session-card.connecting .status-header-bar .status-label {
		color: var(--status-connecting);
	}

	.session-card.attention .status-header-bar .status-indicator {
		background: var(--status-permission);
		box-shadow: 0 0 6px var(--status-permission);
	}

	.session-card.permission .status-header-bar .status-indicator {
		background: var(--status-permission);
		box-shadow: 0 0 6px var(--status-permission);
	}

	.session-card.waiting .status-header-bar .status-indicator {
		background: var(--status-input);
	}

	.session-card.working .status-header-bar .status-indicator {
		background: var(--status-working);
		animation: pulse-indicator 2s ease-in-out infinite;
	}

	.session-card.connecting .status-header-bar .status-indicator {
		background: var(--status-connecting);
		animation: pulse-indicator 2s ease-in-out infinite;
	}

	@keyframes pulse-indicator {
		0%, 100% { opacity: 1; }
		50% { opacity: 0.5; }
	}

	/* Confirmation Dialog */
	.confirm-overlay {
		position: fixed;
		top: 0;
		left: 0;
		right: 0;
		bottom: 0;
		background: rgba(0, 0, 0, 0.6);
		z-index: 9998;
	}

	.confirm-dialog {
		position: fixed;
		top: 50%;
		left: 50%;
		transform: translate(-50%, -50%);
		background: var(--bg-card);
		border: 1px solid var(--border-default);
		border-radius: var(--radius-lg);
		padding: var(--space-lg);
		z-index: 9999;
		min-width: 300px;
		box-shadow: 0 8px 32px rgba(0, 0, 0, 0.8);
	}

	.confirm-message {
		font-family: var(--font-sans);
		font-size: 16px;
		color: var(--text-primary);
		margin: 0 0 var(--space-md) 0;
	}

	.confirm-actions {
		display: flex;
		justify-content: flex-end;
		gap: var(--space-sm);
	}

	.confirm-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 8px 16px;
		font-family: var(--font-mono);
		font-size: 14px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border: 1px solid var(--border-default);
		border-radius: var(--radius-md);
		cursor: pointer;
		transition: all 0.2s ease;
	}

	.confirm-btn.cancel {
		background: var(--bg-base);
		color: var(--text-muted);
	}

	.confirm-btn.cancel:hover {
		background: var(--bg-card-hover);
		color: var(--text-primary);
		border-color: var(--text-muted);
	}

	.confirm-btn.danger {
		background: var(--status-permission);
		color: var(--bg-base);
		border-color: var(--status-permission);
	}

	.confirm-btn.danger:hover {
		background: var(--status-permission);
		opacity: 0.9;
	}

	/* ── Mobile Responsive ─────────────────────────────────────── */
	@media (max-width: 768px) {
		.session-card {
			height: auto;
			min-height: auto;
			padding: var(--space-md);
		}

		.card-main-title {
			font-size: 13px;
		}

		.branch-name {
			max-width: 150px;
		}

		.task-preview {
			font-size: 13px;
			-webkit-line-clamp: 2;
			line-clamp: 2;
		}

		.card-actions {
			flex-wrap: wrap;
		}
	}

</style>
