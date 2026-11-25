/**
 * Imported picture character packs (format: docs/character-pack.md).
 *
 * The 2D engine keeps running its keyboard, mouse, sleep, petting and privacy
 * logic; this renderer picks the pack picture for the moment and brings it to
 * life: it breathes, bends toward the cursor and the key being pressed, nods on
 * every key press, fades between poses, plays the pack's little idle acts now
 * and then, and shows floating Zs while asleep. The bottom of each picture (the
 * paws on the keyboard) stays put; the higher a part is, the more it moves.
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
  /** Short animations played now and then while idle (looking around, yawning...). */
  idleActs?: CharacterStateFrames[];
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
/** Fade between poses; typing frames switch too fast to fade without ghosting. */
const FADE_MS = 140;
const TYPING_FADE_MS = 50;
/** Picture strips drawn per frame for the bend/breathe deformation. */
const STRIPS = 48;
/** Share of the picture height (from the bottom) that never moves: the paws. */
const PLANTED = 0.16;

export interface CharacterFlags {
  privacy: boolean;
  asleep: boolean;
  petting: boolean;
}

interface Particle {
  kind: 'z' | 'heart';
  bornAt: number;
  dx: number;
}

const approach = (rate: number, dt: number) => 1 - Math.exp(-rate * dt);
const smoothstep = (a: number, b: number, x: number) => {
  const t = Math.min(1, Math.max(0, (x - a) / (b - a)));
  return t * t * (3 - 2 * t);
};

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
  private nextBlinkAt = 0;
  private blinkUntil = 0;
  private stateStartedAt = 0;
  private lastState: CharacterStateName | null = null;

  // Idle acts
  private act: CharacterStateFrames | null = null;
  private actStartedAt = 0;
  private nextActAt = 0;

  // Motion
  private cursorX = 0;
  private cursorY = 0;
  private lookX = 0;
  private lookY = 0;
  private keyLean = 0;
  private nod = 0;
  private hop = 0;

  // Cross-fade between pictures
  private current: ImageBitmap | null = null;
  private previous: ImageBitmap | null = null;
  private fadeStartedAt = 0;
  private fadeMs = FADE_MS;
  private currentState: CharacterStateName | null = null;

  private particles: Particle[] = [];
  private nextParticleAt = 0;

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
    this.act = null;
    this.current = null;
    this.previous = null;
    const token = ++this.loadToken;
    const loader = this.loader;
    if (!loader) return;
    const files = new Set<string>();
    for (const s of Object.values(info.states)) s?.frames.forEach((f) => files.add(f));
    for (const a of info.idleActs ?? []) a.frames.forEach((f) => files.add(f));
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
    this.act = null;
    this.current = null;
    this.previous = null;
    this.images.forEach((bmp) => bmp.close());
    this.images.clear();
  }

  public onKey(side: 'left' | 'right', now: number): void {
    this.lastKeyAt = now;
    this.lastKeySide = side;
    this.keyPresses++;
    this.keyLean = Math.max(-1, Math.min(1, this.keyLean + (side === 'left' ? -0.55 : 0.55)));
    this.nod = 1;
    this.act = null;
  }

  public onMouseMove(now: number): void {
    this.lastMouseMoveAt = now;
  }

  /** Cursor relative to the pet window (0..1 inside it). */
  public onCursor(relX: number, relY: number): void {
    this.cursorX = Math.max(-1, Math.min(1, (relX - 0.5) / 1.2));
    this.cursorY = Math.max(-1, Math.min(1, (relY - 0.4) / 1.2));
  }

  public onMouseButton(down: boolean, now: number): void {
    this.mouseDown = down;
    this.lastMouseMoveAt = now;
    if (down) this.hop = Math.max(this.hop, 0.7);
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

  /** The picture to show now: a running idle act, or the state's frame. */
  private pickPicture(now: number, flags: CharacterFlags): ImageBitmap | null {
    const info = this.info!;
    const state = this.resolve(this.pickState(now, flags));
    if (state !== this.lastState) {
      this.lastState = state;
      this.stateStartedAt = now;
      if (state !== 'idle' && state !== 'blink') this.act = null;
      this.nextActAt = Math.max(this.nextActAt, now + 2500);
    }

    const acts = info.idleActs ?? [];
    if (state === 'idle' && acts.length > 0) {
      if (!this.act && now >= this.nextActAt) {
        this.act = acts[Math.floor(Math.random() * acts.length)];
        this.actStartedAt = now;
      }
      if (this.act) {
        const i = Math.floor(((now - this.actStartedAt) / 1000) * this.act.fps);
        if (i < this.act.frames.length) {
          return this.images.get(this.act.frames[i]) ?? null;
        }
        this.act = null;
        this.nextActAt = now + 4000 + Math.random() * 6000;
      }
    }

    const spec = info.states[state];
    if (!spec) return null;
    const index = state.startsWith('typing')
      ? this.keyPresses % spec.frames.length
      : Math.floor(((now - this.stateStartedAt) / 1000) * spec.fps) % spec.frames.length;
    return this.images.get(spec.frames[index]) ?? null;
  }

  /**
   * Draws `bmp` as horizontal strips: the planted bottom stays, higher strips
   * shift sideways by `lean` px (a smooth bend) and stretch by `stretch`
   * (breathing / nodding), all anchored at the bottom centre.
   */
  private drawBent(
    ctx: CanvasRenderingContext2D,
    bmp: ImageBitmap,
    x: number,
    bottom: number,
    w: number,
    h: number,
    lean: number,
    lift: number,
    stretch: number,
  ): void {
    const sh = bmp.height / STRIPS;
    const heightAt = (v: number) => {
      const g = smoothstep(PLANTED, 1, v);
      return h * (v + stretch * g) + lift * g;
    };
    for (let i = 0; i < STRIPS; i++) {
      const vTop = 1 - i / STRIPS;
      const vBottom = 1 - (i + 1) / STRIPS;
      const yTop = bottom - heightAt(vTop);
      const yBottom = bottom - heightAt(vBottom);
      const bend = smoothstep(PLANTED, 1, (vTop + vBottom) / 2);
      ctx.drawImage(bmp, 0, i * sh, bmp.width, sh, x + lean * bend * bend, yTop, w, yBottom - yTop + 0.6);
    }
  }

  private drawParticles(ctx: CanvasRenderingContext2D, now: number, headX: number, headY: number): void {
    this.particles = this.particles.filter((p) => now - p.bornAt < 2600);
    for (const p of this.particles) {
      const t = (now - p.bornAt) / 2600;
      const alpha = Math.sin(Math.PI * Math.min(1, t * 1.15));
      ctx.save();
      ctx.globalAlpha = Math.max(0, alpha);
      ctx.translate(headX + p.dx + t * 26, headY - t * 46);
      if (p.kind === 'z') {
        ctx.font = `700 ${12 + t * 10}px "Avenir Next", "PingFang SC", sans-serif`;
        ctx.lineWidth = 3;
        ctx.strokeStyle = 'rgba(255,255,255,0.9)';
        ctx.fillStyle = '#7d93e8';
        ctx.strokeText('z', 0, 0);
        ctx.fillText('z', 0, 0);
      } else {
        const s = 5 + t * 3;
        ctx.fillStyle = '#ff7fa0';
        ctx.beginPath();
        ctx.moveTo(0, s * 0.9);
        ctx.bezierCurveTo(-s * 1.4, -s * 0.2, -s * 0.6, -s * 1.3, 0, -s * 0.45);
        ctx.bezierCurveTo(s * 0.6, -s * 1.3, s * 1.4, -s * 0.2, 0, s * 0.9);
        ctx.fill();
      }
      ctx.restore();
    }
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
    const bmp = this.pickPicture(now, flags);
    if (!bmp) return;
    if (bmp !== this.current) {
      const wasTyping = this.currentState?.startsWith('typing') ?? false;
      const isTyping = this.lastState?.startsWith('typing') ?? false;
      this.fadeMs = wasTyping && isTyping ? 0 : wasTyping || isTyping ? TYPING_FADE_MS : FADE_MS;
      this.previous = this.fadeMs > 0 ? this.current : null;
      this.current = bmp;
      this.currentState = this.lastState;
      this.fadeStartedAt = now;
    }

    // Motion springs
    const typing = now - this.lastKeyAt < TYPING_HOLD_MS;
    const lookTargetX = flags.asleep || flags.privacy ? 0 : this.cursorX;
    const lookTargetY = flags.asleep || flags.privacy ? 0 : this.cursorY;
    this.lookX += (lookTargetX - this.lookX) * approach(typing ? 3 : 6, dt);
    this.lookY += (lookTargetY - this.lookY) * approach(6, dt);
    this.keyLean += (0 - this.keyLean) * approach(4.5, dt);
    this.nod = Math.max(0, this.nod - dt * 7.5);
    this.hop = Math.max(0, this.hop - dt * 6);

    const t = now / 1000;
    const asleep = flags.asleep;
    const breath = asleep ? Math.sin(t * 1.35) * 0.026 : Math.sin(t * 2.1) * 0.012;
    const sway = Math.sin(t * 0.55) * (asleep ? 0.6 : 2.2) + (flags.petting ? Math.sin(t * 5.5) * 3 : 0);
    const nodDip = -0.045 * Math.sin(Math.min(1, this.nod) * Math.PI);
    const lean = this.lookX * 9 + this.keyLean * 6 + sway;
    const lift = -this.lookY * 5 + this.hop * 3;

    const box = CHARACTER_BOX * info.scale;
    const fit = Math.min(box / bmp.width, box / bmp.height);
    const w = bmp.width * fit;
    const h = bmp.height * fit;
    const x = anchorX + info.offsetX - w / 2;
    const bottom = anchorY + info.offsetY;
    const stretch = breath + nodDip;

    const fade = this.fadeMs > 0 ? Math.min(1, (now - this.fadeStartedAt) / this.fadeMs) : 1;
    if (this.previous && fade < 1) {
      this.drawBent(ctx, this.previous, x, bottom, w, h, lean, lift, stretch);
      ctx.save();
      ctx.globalAlpha = fade;
      this.drawBent(ctx, bmp, x, bottom, w, h, lean, lift, stretch);
      ctx.restore();
    } else {
      this.previous = null;
      this.drawBent(ctx, bmp, x, bottom, w, h, lean, lift, stretch);
    }

    // Floating Zs while asleep, hearts while being petted.
    const kind = asleep ? 'z' : flags.petting ? 'heart' : null;
    if (kind && now >= this.nextParticleAt) {
      this.particles.push({ kind, bornAt: now, dx: (Math.random() - 0.3) * 14 });
      this.nextParticleAt = now + (kind === 'z' ? 900 : 520);
    }
    if (this.particles.length > 0) {
      this.drawParticles(ctx, now, x + w * 0.72 + lean, bottom - h * 0.86);
    }
  }
}
