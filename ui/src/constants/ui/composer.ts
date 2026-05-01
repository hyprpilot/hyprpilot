/**
 * Composer pill kinds. `resource` pills expand inline at submit time;
 * `attachment` pills ride on the next turn as ACP content blocks
 * (image / audio / blob) and `data` carries the base64 payload.
 */
export enum ComposerPillKind {
  Resource = 'resource',
  Attachment = 'attachment'
}
