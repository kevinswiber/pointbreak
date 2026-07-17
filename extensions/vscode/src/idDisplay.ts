const SHORT_ID_LENGTH = 12;

/** Compact a typed content-addressed reference for human-facing labels. */
export function shortReferenceId(id: string): string {
  const value = id.split(":").at(-1) ?? id;
  return value.slice(0, SHORT_ID_LENGTH);
}

interface RevisionDiscoveryEntry {
  revisionId: string;
  summary?: string;
  mergeStatus: string;
}

/** Primary and secondary labels for a revision discovery row or picker. */
export function revisionDiscoveryDisplay(entry: RevisionDiscoveryEntry): {
  label: string;
  description: string;
} {
  const shortId = shortReferenceId(entry.revisionId);
  return {
    label: entry.summary || shortId,
    description: entry.summary
      ? `${shortId} · ${entry.mergeStatus}`
      : entry.mergeStatus,
  };
}
