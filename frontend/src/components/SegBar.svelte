<script>
  // Segmented progress drawn like wet pigment: committed ranges as a soft wash
  // underneath, one coloured lane per active connection on top (its piece so
  // far, plus a glowing tip at its position).
  let { total = 0, done = 0, doneRanges = [], segments = [], state = '', verified = 0 } = $props()

  const hues = [228, 184, 40, 340, 262, 160, 20, 200, 300, 90, 10, 140, 250, 60, 320, 120]
  const pct = (x) => (total > 0 ? Math.max(0, Math.min(100, (x / total) * 100)) : 0)
  let segs = $derived([...(segments || [])].sort((a, b) => a.s - b.s))
</script>

<div class="bar" class:done={state === 'done'} class:err={state === 'error'} class:paused={state === 'paused' || state === 'needs_link'} role="progressbar" aria-valuemin="0" aria-valuemax={total} aria-valuenow={done}>
  {#if total > 0}
    {#each doneRanges || [] as r}
      <div class="range" style="left:{pct(r.Start)}%;width:{pct(r.End) - pct(r.Start)}%"></div>
    {/each}
    {#each segs as s, i}
      <div class="seg" class:verifying={s.v} style="left:{pct(s.s)}%;width:{Math.max(0.15, pct(s.p) - pct(s.s))}%;--h:{hues[i % hues.length]}"></div>
      <div class="head" style="left:{pct(s.p)}%;--h:{hues[i % hues.length]}"></div>
    {/each}
    {#if state === 'verifying'}
      <div class="verify" style="width:{pct(verified)}%"></div>
    {/if}
  {:else if state === 'downloading'}
    <div class="indeterminate"></div>
  {/if}
</div>

<style>
  .bar { position: relative; height: 9px; border-radius: 99px; background: var(--track); overflow: hidden; box-shadow: 0 1px 1px rgb(28 34 51 / 6%) inset; }
  .range { position: absolute; top: 0; bottom: 0; background: var(--seg-done); }
  .done .range { background: linear-gradient(90deg, var(--teal), color-mix(in oklab, var(--teal) 60%, var(--done))); }
  .err .range { background: color-mix(in oklab, var(--err) 45%, var(--track)); }
  .paused .range { background: color-mix(in oklab, var(--muted) 45%, var(--track)); }
  .seg { position: absolute; top: 0; bottom: 0; background: linear-gradient(90deg, hsl(var(--h) 60% 62% / 0.75), hsl(var(--h) 70% 52%)); }
  .seg.verifying { background: repeating-linear-gradient(45deg, hsl(var(--h) 65% 55%) 0 3px, hsl(var(--h) 65% 55% / 0.35) 3px 6px); }
  .head { position: absolute; top: 0; bottom: 0; width: 3px; margin-left: -1.5px; background: hsl(var(--h) 90% 40%); box-shadow: 0 0 6px 1px hsl(var(--h) 90% 55% / 0.7); }
  .verify { position: absolute; top: 0; bottom: 0; left: 0; background: var(--done); opacity: 0.9; transition: width 0.3s; }
  .indeterminate { position: absolute; top: 0; bottom: 0; width: 30%; border-radius: 99px; background: linear-gradient(90deg, transparent, var(--accent), transparent); animation: slide 1.3s infinite ease-in-out; }
  @keyframes slide { from { left: -30%; } to { left: 100%; } }
</style>
