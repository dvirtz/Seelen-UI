import { invoke, SeelenCommand } from "@seelen-ui/lib";
import type { DesktopWorkspace, VirtualDesktops } from "@seelen-ui/lib/types";
import { Button, Input, message, Modal, Select, Switch } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { SettingsGroup, SettingsOption } from "../../../components/SettingsBox/index.tsx";
import { $virtual_desktops } from "../../shared/signals.ts";
import {
  getShareWorkspacesBetweenMonitors,
  getSwitchWorkspacesTogether,
  setShareWorkspacesBetweenMonitors,
  setSwitchWorkspacesTogether,
} from "../application.ts";
import cs from "./WorkspaceSettings.module.css";

export function WorkspaceSettings() {
  const { t } = useTranslation();
  const state = $virtual_desktops.value;
  const ids = Object.keys(state.monitors).sort((a, b) =>
    Number(state.connectedMonitors.includes(b)) - Number(state.connectedMonitors.includes(a)) ||
    (state.monitorNames[a] || a).localeCompare(state.monitorNames[b] || b) || a.localeCompare(b)
  );
  return (
    <section className={cs.section} aria-label={t("workspace_settings.title")}>
      <h2>{t("workspace_settings.title")}</h2>
      <SettingsGroup>
        <SettingsOption
          label={t("workspace_settings.share")}
          description={t("workspace_settings.share_description")}
          action={
            <Switch
              checked={getShareWorkspacesBetweenMonitors()}
              onChange={setShareWorkspacesBetweenMonitors}
              aria-label={t("workspace_settings.share")}
            />
          }
        />
        <SettingsOption
          label={t("workspace_settings.switch_together")}
          description={t("workspace_settings.switch_together_description")}
          action={
            <Switch
              checked={getSwitchWorkspacesTogether()}
              onChange={setSwitchWorkspacesTogether}
              aria-label={t("workspace_settings.switch_together")}
            />
          }
        />
      </SettingsGroup>
      <p>{t("workspace_settings.immediate")}</p>
      {ids.length === 0 && <p>{t("workspace_settings.empty")}</p>}
      {ids.map((id) => <MonitorWorkspaces key={id} monitorId={id} state={state} />)}
    </section>
  );
}

function MonitorWorkspaces({ monitorId, state }: { monitorId: string; state: VirtualDesktops }) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [source, setSource] = useState<string | undefined>();
  const [editing, setEditing] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [modal, contextHolder] = Modal.useModal();
  const monitor = state.monitors[monitorId]!;
  const connected = state.connectedMonitors.includes(monitorId);
  const name = (id: string) => state.monitorNames[id] || id;
  const gridKey = JSON.stringify(monitor.workspaceIds);
  const shared = Object.keys(state.monitors).filter((id) =>
    id !== monitorId && JSON.stringify(state.monitors[id]!.workspaceIds) === gridKey
  );
  const choices = Object.keys(state.monitors).filter((id) =>
    id !== monitorId && JSON.stringify(state.monitors[id]!.workspaceIds) !== gridKey
  );
  const selectedSource = source && choices.includes(source) ? source : undefined;
  const label = (workspace: DesktopWorkspace, row: number, column: number) =>
    workspace.name || t("workspace_settings.position", { row: row + 1, column: column + 1 });
  const selected = editing ? state.workspaces[editing] : undefined;
  const row = monitor.workspaceIds.findIndex((cells) => cells.includes(editing || ""));
  const column = monitor.workspaceIds[row]?.indexOf(editing || "") ?? -1;
  const columns = monitor.workspaceIds[0]?.length || 1;
  const removeColumn = columns > 1;
  const canRemove = columns > 1 || monitor.workspaceIds.length > 1;

  async function perform(action: () => Promise<unknown>): Promise<boolean> {
    setBusy(true);
    try {
      await action();
      return true;
    } catch (error) {
      console.error(error);
      message.error(t("workspace_settings.action_failed"));
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function assign(sourceMonitorId: string | null) {
    const confirmed = await modal.confirm({
      title: t(sourceMonitorId ? "workspace_settings.reuse" : "workspace_settings.new_grid"),
      content: sourceMonitorId
        ? t("workspace_settings.reuse_confirm", { monitor: name(sourceMonitorId) })
        : t("workspace_settings.new_confirm"),
      okText: t("workspace_settings.apply"),
      cancelText: t("cancel"),
      centered: true,
    });
    if (confirmed) {
      if (await perform(() => invoke(SeelenCommand.SetMonitorWorkspaceGrid, { monitorId, sourceMonitorId }))) {
        setSource(undefined);
      }
    }
  }

  async function remove() {
    if (!selected || !canRemove) return;
    const confirmed = await modal.confirm({
      title: t(removeColumn ? "workspace_settings.remove_column" : "workspace_settings.remove_row"),
      content: t("workspace_settings.remove_confirm"),
      okText: t("delete"),
      okButtonProps: { danger: true },
      cancelText: t("cancel"),
      centered: true,
    });
    if (confirmed && await perform(() => invoke(SeelenCommand.DestroyWorkspace, { workspaceId: selected.id }))) {
      setEditing(null);
    }
  }

  return (
    <SettingsGroup>
      {contextHolder}
      <article className={cs.monitor} aria-label={name(monitorId)}>
        <div className={cs.heading}>
          <b className={cs.identity} title={monitorId}>{name(monitorId)}</b>
          <span>{t(connected ? "workspace_settings.connected" : "workspace_settings.disconnected")}</span>
        </div>
        {name(monitorId) !== monitorId && <small className={cs.identity}>{monitorId}</small>}
        <p>
          {shared.length
            ? t("workspace_settings.shared_with", { monitors: shared.map((id) => `${name(id)} (${id})`).join(", ") })
            : t("workspace_settings.independent")}
        </p>
        <div className={cs.actions}>
          <Select
            className={cs.choice}
            aria-label={t("workspace_settings.choose_grid")}
            placeholder={t("workspace_settings.choose_grid")}
            value={selectedSource}
            disabled={busy || !choices.length}
            onChange={setSource}
            options={choices.map((id) => ({
              value: id,
              label: `${name(id)} (${id}) — ${
                t(
                  state.connectedMonitors.includes(id)
                    ? "workspace_settings.connected"
                    : "workspace_settings.disconnected",
                )
              }`,
            }))}
          />
          <Button disabled={busy || !selectedSource} onClick={() => assign(selectedSource!)}>
            {t("workspace_settings.reuse")}
          </Button>
          <Button disabled={busy} onClick={() => assign(null)}>{t("workspace_settings.new_grid")}</Button>
        </div>
        <div className={cs.grid} aria-label={t("workspace_settings.grid")}>
          {monitor.workspaceIds.map((cells, row) => (
            <div
              className={cs.row}
              key={row}
              style={{ gridTemplateColumns: `repeat(${cells.length}, minmax(120px, 1fr))` }}
            >
              {cells.map((id, column) => {
                const workspace = state.workspaces[id];
                if (!workspace) return null;
                return (
                  <Button
                    className={cs.cell}
                    key={id}
                    disabled={busy}
                    title={id}
                    onClick={() => {
                      setEditing(id);
                      setDraftName(workspace.name || "");
                    }}
                  >
                    {label(workspace, row, column)}
                    {id === monitor.active_workspace && ` (${t("workspace_settings.active")})`}
                  </Button>
                );
              })}
            </div>
          ))}
        </div>
        <div className={cs.actions}>
          <Button disabled={busy} onClick={() => perform(() => invoke(SeelenCommand.CreateWorkspace, { monitorId }))}>
            {t("workspace_settings.add_column")}
          </Button>
          <Button
            disabled={busy}
            onClick={() => perform(() => invoke(SeelenCommand.CreateWorkspaceRow, { monitorId }))}
          >
            {t("workspace_settings.add_row")}
          </Button>
        </div>
        <Modal
          open={!!selected && row >= 0}
          title={selected ? label(selected, row, column) : ""}
          onCancel={() => setEditing(null)}
          footer={null}
          centered
        >
          {selected && (
            <div className={cs.editor}>
              <label htmlFor={`workspace-name-${monitorId}`}>{t("workspace_settings.name")}</label>
              <Input
                id={`workspace-name-${monitorId}`}
                value={draftName}
                disabled={busy}
                onChange={(event) => setDraftName(event.currentTarget.value)}
                placeholder={t("workspace_settings.position", { row: row + 1, column: column + 1 })}
              />
              {!!shared.length && <p>{t("workspace_settings.shared_edits")}</p>}
              <div className={cs.actions}>
                <Button
                  disabled={busy}
                  onClick={async () => {
                    if (
                      await perform(() =>
                        invoke(SeelenCommand.RenameWorkspace, {
                          workspaceId: selected.id,
                          name: draftName.trim() || null,
                        })
                      )
                    ) setEditing(null);
                  }}
                >
                  {t("save")}
                </Button>
                <Button
                  disabled={busy || !connected || selected.id === monitor.active_workspace}
                  onClick={async () => {
                    if (await perform(() => invoke(SeelenCommand.SwitchWorkspace, { workspaceId: selected.id }))) {
                      setEditing(null);
                    }
                  }}
                >
                  {t("workspace_settings.activate")}
                </Button>
                <Button disabled={busy || !canRemove} onClick={remove}>
                  {t(removeColumn ? "workspace_settings.remove_column" : "workspace_settings.remove_row")}
                </Button>
              </div>
            </div>
          )}
        </Modal>
      </article>
    </SettingsGroup>
  );
}
