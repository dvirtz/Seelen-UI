import { settings } from "../../state/mod.ts";

export function getShareWorkspacesBetweenMonitors(): boolean {
  return settings.value.shareWorkspacesBetweenMonitors;
}

export function setShareWorkspacesBetweenMonitors(shareWorkspacesBetweenMonitors: boolean) {
  settings.value = { ...settings.value, shareWorkspacesBetweenMonitors };
}

export function getSwitchWorkspacesTogether(): boolean {
  return settings.value.switchWorkspacesTogether;
}

export function setSwitchWorkspacesTogether(switchWorkspacesTogether: boolean) {
  settings.value = { ...settings.value, switchWorkspacesTogether };
}
