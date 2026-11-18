export interface ManifestInput {
  version: string;
  arch: string;
  url: string;
  signature: string;
  pubDate: string;
}
export interface Manifest {
  version: string;
  pub_date: string;
  platforms: Record<string, { signature: string; url: string }>;
}
export function platformKey(arch: string): string;
export function manifest(input: ManifestInput): Manifest;
