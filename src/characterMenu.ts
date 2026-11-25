import { invoke } from '@tauri-apps/api/core';
import type { CatSkinConfig } from './catSkins';

const DELETE_LABEL = '删除';
const DELETE_CONFIRM_LABEL = '再点一次删除';
let deleteArmedFor: string | null = null;
let disarmTimer: number | null = null;

function disarmDelete(): void {
  deleteArmedFor = null;
  if (disarmTimer !== null) {
    window.clearTimeout(disarmTimer);
    disarmTimer = null;
  }
  const btn = document.getElementById('btn-delete-character');
  if (btn) {
    btn.textContent = DELETE_LABEL;
    btn.classList.remove('danger');
  }
}

/** Shows export/delete only while an imported character is active. */
export function syncCharacterMenu(active: CatSkinConfig | undefined): void {
  const imported = Boolean(active?.character);
  document.getElementById('btn-export-character')?.classList.toggle('hidden', !imported);
  document.getElementById('btn-delete-character')?.classList.toggle('hidden', !imported);
  if (deleteArmedFor !== active?.id) disarmDelete();
}

/**
 * Runs a character-pack menu action. `beforeDialog` closes the menu that is
 * not a separate window (the pet window's built-in menu). Returns false when
 * `action` is not one of ours.
 */
export function handleCharacterAction(
  action: string | undefined,
  active: CatSkinConfig | undefined,
  beforeDialog: () => void = () => {},
): boolean {
  switch (action) {
    case 'import-character':
      beforeDialog();
      void invoke('import_character_pack').catch(() => {});
      return true;
    case 'photo-character':
      beforeDialog();
      void invoke('generate_character_from_photo').catch(() => {});
      return true;
    case 'export-character':
      if (active?.character) {
        beforeDialog();
        void invoke('export_character_pack', { id: active.id }).catch(() => {});
      }
      return true;
    case 'delete-character': {
      if (!active?.character) return true;
      const btn = document.getElementById('btn-delete-character');
      if (deleteArmedFor !== active.id) {
        deleteArmedFor = active.id;
        if (btn) {
          btn.textContent = DELETE_CONFIRM_LABEL;
          btn.classList.add('danger');
        }
        disarmTimer = window.setTimeout(disarmDelete, 6000);
        return true;
      }
      disarmDelete();
      void invoke('delete_character_pack', { id: active.id });
      return true;
    }
    default:
      return false;
  }
}
