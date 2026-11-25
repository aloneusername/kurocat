/**
 * Imported picture character packs (format: docs/character-pack.md).
 *
 * The 2D engine keeps running its keyboard, mouse, sleep, petting and privacy
 * logic; this renderer only decides which picture of the pack to show and draws
 * it where the cat would sit.
 */

export interface CharacterStateFrames {
  frames: string[];
  fps: number;
}

export interface CharacterInfo {
  rev: string;
  author?: string;
  scale: number;
  offsetX: number;
  offsetY: number;
  showKeyboard: boolean;
  purr: boolean;
  states: Partial<Record<CharacterStateName, CharacterStateFrames>>;
}

export type CharacterStateName =
  | 'idle'
  | 'blink'
  | 'typing'
  | 'typingLeft'
  | 'typingRight'
  | 'mouse'
  | 'mouseClick'
  | 'privacy'
  | 'sleep'
  | 'pet';

/** Reads one picture of a pack (desktop: Tauri command, OBS: HTTP). */
export type CharacterAssetLoader = (id: string, file: string, rev: string) => Promise<Blob>;

/** Where each state falls back to when a pack does not draw it. */
const FALLBACKS: Record<CharacterStateName, CharacterStateName[]> = {
  idle: [],
  blink: ['idle'],
  typing: ['idle'],
  typingLeft: ['typing', 'idle'],
  typingRight: ['typing', 'idle'],
  mouse: ['idle'],
  mouseClick: ['mouse', 'idle'],
  privacy: ['blink', 'idle'],
  sleep: ['blink', 'idle'],
  pet: ['idle'],
};

/** Side length, in logical stage pixels, of the box a picture is fitted into. */
export const CHARACTER_BOX = 280;
const TYPING_HOLD_MS = 260;
const MOUSE_HOLD_MS = 450;
const BLINK_MS = 140;

export interface CharacterFlags {
  privacy: boolean;
  asleep: boolean;
  petting: boolean;
}

export class SpriteCharacterRenderer {
  private loader: CharacterAssetLoader | null = null;
  private id: string | null = null;
  private info: CharacterInfo | null = null;
  private images = new Map<string, ImageBitmap>();
  private loadToken = 0;

  private lastKeyAt = -1e9;
  private lastKeySide: 'left' | 'right' = 'left';
  private keyPresses = 0;
  private lastMouseMoveAt = -1e9;
  private mouseDown = false;
  private bob = 0;
  private nextBlinkAt = 0;
  private blinkUntil = 0;
  private stateStartedAt = 0;
  private lastState: CharacterStateName | null = null;

  public setAssetLoader(loader: CharacterAssetLoader): void {
    this.loader = loader;
  }


  public get showKeyboard(): boolean {
    return this.info?.showKeyboard ?? true;
  }

  public get purr(): boolean {
    return this.info?.purr ?? false;
  }

  /** Switches pack; resolves once its pictures are loaded (or failed). */
  public async setCharacter(id: string, info: CharacterInfo, onReady: () => void): Promise<void> {
    if (this.id === id && this.info?.rev === info.rev && this.images.size > 0) return;
    this.id = id;
    this.info = info;
    this.lastState = null;
    const token = ++this.loadToken;
    const loader = this.loader;
    if (!loader) return;
    const files = new Set<string>();
    for (const s of Object.values(info.states)) s?.frames.forEach((f) => files.add(f));
    const next = new Map<string, ImageBitmap>();
    await Promise.all(
      [...files].map(async (file) => {
        try {
          next.set(file, await createImageBitmap(await loader(id, file, info.rev)));
        } catch (err) {
          console.warn(`[kurocat] character picture failed to load: ${file}`, err);
        }
      }),
    );
    if (token !== this.loadToken) {
      next.forEach((bmp) => bmp.close());
      return;
    }
    this.images.forEach((bmp) => bmp.close());
    this.images = next;
    onReady();
  }

  public clear(): void {
    this.loadToken++;
    this.id = null;
    this.info = null;
    this.images.forEach((bmp) => bmp.close());
    this.images.clear();
  }

  public onKey(side: 'left' | 'right', now: number): void {
    this.lastKeyAt = now;
    this.lastKeySide = side;
    this.keyPresses++;
    this.bob = 1;
  }

  public onMouseMove(now: number): void {
    this.lastMouseMoveAt = now;
  }

  public onMouseButton(down: boolean, now: number): void {
    this.mouseDown = down;
    this.lastMouseMoveAt = now;
    if (down) this.bob = Math.max(this.bob, 0.6);
  }

  private has(state: CharacterStateName): boolean {
    return Boolean(this.info?.states[state]);
  }

  private resolve(state: CharacterStateName): CharacterStateName {
    if (this.has(state)) return state;
    return FALLBACKS[state].find((s) => this.has(s)) ?? 'idle';
  }

  private pickState(now: number, flags: CharacterFlags): CharacterStateName {
    if (flags.privacy) return 'privacy';
    if (this.mouseDown) return 'mouseClick';
    if (now - this.lastKeyAt < TYPING_HOLD_MS) {
      return this.lastKeySide === 'left' ? 'typingLeft' : 'typingRight';
    }
    if (flags.petting) return 'pet';
    if (flags.asleep) return 'sleep';
    if (now - this.lastMouseMoveAt < MOUSE_HOLD_MS) return 'mouse';
    if (now >= this.nextBlinkAt) {
      this.blinkUntil = now + BLINK_MS;
      this.nextBlinkAt = now + 3000 + Math.random() * 3200;
    }
    if (now < this.blinkUntil) return 'blink';
    return 'idle';
  }

  /**
   * Draws the current picture with its bottom centre at (anchorX, anchorY) in
   * logical stage pixels.
   */
  public draw(
    ctx: CanvasRenderingContext2D,
    now: number,
    dt: number,
    anchorX: number,
    anchorY: number,
    flags: CharacterFlags,
  ): void {
    const info = this.info;
    if (!info || this.images.size === 0) return;
    const state = this.resolve(this.pickState(now, flags));
    if (state !== this.lastState) {
      this.lastState = state;
      this.stateStartedAt = now;
    }
    const spec = info.states[state];
    if (!spec) return;
    const index = state.startsWith('typing')
      ? this.keyPresses % spec.frames.length
      : Math.floor(((now - this.stateStartedAt) / 1000) * spec.fps) % spec.frames.length;
    const bmp = this.images.get(spec.frames[index]);
    if (!bmp) return;

    this.bob = Math.max(0, this.bob - dt * 7);
    const box = CHARACTER_BOX * info.scale;
    const fit = Math.min(box / bmp.width, box / bmp.height);
    const w = bmp.width * fit;
    const h = bmp.height * fit;
    const x = anchorX + info.offsetX - w / 2;
    const y = anchorY + info.offsetY - h + this.bob * 3;
    ctx.drawImage(bmp, x, y, w, h);
  }
}
