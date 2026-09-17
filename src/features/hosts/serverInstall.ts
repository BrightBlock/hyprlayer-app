import { Channel } from "@tauri-apps/api/core";

import { commands, type Host, type ProvisionStep } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";

export const STEP_LABEL: Record<ProvisionStep, string> = {
  probe: "Checking the host",
  download: "Downloading the server",
  verify: "Verifying the download",
  upload: "Copying to the host",
  install: "Installing",
  done: "Installed",
};

/** The failure a host can be provisioned out of; anything else is not ours to fix. */
export const NO_SERVER = "remote_cli_missing";

/** Channels are single-use, so every run mints its own. */
export async function runProvision(
  host: Host,
  onStep: (step: ProvisionStep) => void,
): Promise<void> {
  const channel = new Channel<ProvisionStep>();
  channel.onmessage = onStep;
  await unwrap(commands.provisionHost(host, channel));
}
