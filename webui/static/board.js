// Shared parts of the drag-and-drop boards on the profile page (apps, web
// pages): DOM helpers, an undo toast, popover menus, and dragging tiles with a
// mouse, pen or finger.
window.SGBoard = (() => {
  const fmt = (s, o) => s.replace(/\{(\w+)\}/g, (_, k) => o[k] ?? '');
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };
  const hue = s => { let h = 0; for (const c of s) h = (h * 31 + c.codePointAt(0)) >>> 0; return h % 360; };

  // ── toast with an optional Undo ────────────────────────────────────────
  let toastEl = null, toastTimer;
  function toast(msg, undo, undoLabel) {
    if (!toastEl) { toastEl = el('div', 'sg-toast'); toastEl.setAttribute('role', 'status'); document.body.append(toastEl); }
    toastEl.replaceChildren(el('span', null, msg));
    if (undo) { const b = el('button', null, undoLabel); b.type = 'button'; b.onclick = () => { toastEl.hidden = true; undo(); }; toastEl.append(b); }
    toastEl.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { toastEl.hidden = true; }, 6000);
  }

  // ── menus: the click / tap / keyboard way to do what dragging does ─────
  let pop = null;
  const closePop = () => { pop?.remove(); pop = null; };
  function openPop(anchor, build) {
    closePop();
    pop = el('div', 'sg-pop'); pop.setAttribute('role', 'menu');
    build(pop);
    document.body.append(pop);
    const r = anchor.getBoundingClientRect();
    const maxLeft = window.scrollX + document.documentElement.clientWidth - pop.offsetWidth - 8;
    pop.style.left = Math.max(8, Math.min(window.scrollX + r.left, maxLeft)) + 'px';
    // below the anchor, or above it when it wouldn't fit on screen
    const below = r.bottom + 6 + pop.offsetHeight <= window.innerHeight;
    pop.style.top = (window.scrollY + (below ? r.bottom + 6 : Math.max(8, r.top - 6 - pop.offsetHeight))) + 'px';
    pop.querySelector('button')?.focus();
  }
  const item = (label, fn, cls) => { const b = el('button', cls || '', label); b.type = 'button'; b.setAttribute('role', 'menuitem'); b.onclick = () => { closePop(); fn(); }; return b; };

  // ── dragging: pointer events for mouse/pen; touch events for fingers (a
  // pointer would be cancelled as soon as the browser starts to scroll) ────
  const boards = [];   // [{root, onTap, dropTarget, onDrop}]
  let drag = null;     // {board, tile, ghost, target, x, y}
  let press = null;    // a press that may become a drag or a click / tap
  const boardOf = t => boards.find(b => b.root.contains(t));

  // `root` holds `.sg-tile`s; a tap opens `onTap(tile)`; while dragging,
  // `dropTarget(elementUnderFinger, drag)` names {kind: 'tile' | 'zone', el} or
  // null; `onDrop(drag)` gets {tile, target} at the end.
  function draggable(root, opts) {
    const b = { root, ...opts };
    boards.push(b);
    root.addEventListener('pointerdown', e => {
      if (e.pointerType === 'touch') return;
      const t = e.target.closest('.sg-tile');
      if (!t || e.button > 0) return;
      press = { b, t, x: e.clientX, y: e.clientY, id: e.pointerId, touch: false };
    });
    // touch: a long press picks the tile up; moving before that is a scroll
    root.addEventListener('touchstart', e => {
      const t = e.target.closest('.sg-tile');
      if (!t || e.touches.length > 1) return;
      const p = e.touches[0];
      press = { b, t, x: p.clientX, y: p.clientY, touch: true, timer: setTimeout(() => press && startDrag(press.x, press.y), 350) };
    }, { passive: true });
    root.addEventListener('contextmenu', e => { if (e.target.closest('.sg-tile')) e.preventDefault(); });
  }

  window.addEventListener('pointermove', e => {
    if (e.pointerType === 'touch') return;
    if (drag) { e.preventDefault(); moveDrag(e.clientX, e.clientY); return; }
    if (press && !press.touch && e.pointerId === press.id && Math.hypot(e.clientX - press.x, e.clientY - press.y) > 5) {
      startDrag(e.clientX, e.clientY);
    }
  }, { passive: false });
  window.addEventListener('pointerup', e => {
    if (e.pointerType === 'touch') return;
    if (drag) { endDrag(); return; }
    if (press && !press.touch && e.pointerId === press.id) { const { b, t } = press; press = null; b.onTap(t); }
  });
  window.addEventListener('touchmove', e => {
    const p = e.touches[0];
    if (drag) { e.preventDefault(); moveDrag(p.clientX, p.clientY); return; }
    if (press?.touch && Math.hypot(p.clientX - press.x, p.clientY - press.y) > 8) { clearTimeout(press.timer); press = null; }
  }, { passive: false });
  window.addEventListener('touchend', e => {
    if (drag) { e.preventDefault(); endDrag(); return; }
    if (press?.touch) { clearTimeout(press.timer); const { b, t } = press; press = null; e.preventDefault(); b.onTap(t); }
  }, { passive: false });
  window.addEventListener('touchcancel', () => { if (press?.timer) clearTimeout(press.timer); press = null; cleanupDrag(); });

  document.addEventListener('pointerdown', e => { if (pop && !pop.contains(e.target) && !e.target.closest('.sg-tile')) closePop(); });
  document.addEventListener('keydown', e => {
    if (e.key === 'Escape') { closePop(); cleanupDrag(); return; }
    const t = e.target.closest?.('.sg-tile');
    if (t && !t.classList.contains('sg-ghost') && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); boardOf(t)?.onTap(t); }
  });

  function startDrag(x, y) {
    const { b, t } = press;
    if (press.timer) clearTimeout(press.timer);
    press = null;
    closePop();
    const ghost = t.cloneNode(true);
    ghost.classList.add('sg-ghost');
    ghost.style.width = t.offsetWidth + 'px';
    document.body.append(ghost);
    t.classList.add('sg-dragging');
    document.body.classList.add('sg-is-dragging');
    drag = { board: b, tile: t, ghost, target: null, x, y, dx: t.offsetWidth / 2, dy: t.offsetHeight / 2 };
    if (navigator.vibrate) navigator.vibrate(10);
    moveDrag(x, y);
    requestAnimationFrame(autoScroll);
  }

  function moveDrag(x, y) {
    drag.x = x; drag.y = y;
    drag.ghost.style.transform = `translate(${x - drag.dx}px, ${y - drag.dy}px) rotate(-3deg)`;
    const under = document.elementFromPoint(x, y);
    const tgt = under ? drag.board.dropTarget(under, drag) : null;
    if (tgt?.el !== drag.target?.el) {
      drag.target?.el.classList.remove('sg-over', 'sg-target');
      tgt?.el.classList.add(tgt.kind === 'tile' ? 'sg-target' : 'sg-over');
      drag.target = tgt;
    }
  }

  function autoScroll() {   // near the window edge while dragging: scroll
    if (!drag) return;
    // the sticky nav covers the top of the page, so the top edge starts below it
    const edge = 70, h = window.innerHeight, top = (document.querySelector('.sticky-top')?.getBoundingClientRect().bottom || 0) + edge;
    if (drag.y < top) window.scrollBy(0, -Math.ceil((top - drag.y) / 4));
    else if (drag.y > h - edge) window.scrollBy(0, Math.ceil((drag.y - (h - edge)) / 4));
    moveDrag(drag.x, drag.y);
    requestAnimationFrame(autoScroll);
  }

  function cleanupDrag() {
    if (!drag) return null;
    const d = drag; drag = null;
    d.ghost.remove();
    d.tile.classList.remove('sg-dragging');
    d.target?.el.classList.remove('sg-over', 'sg-target');
    document.body.classList.remove('sg-is-dragging');
    return d;
  }

  function endDrag() {
    const d = cleanupDrag();
    if (d?.target) d.board.onDrop(d);
  }

  return { fmt, el, hue, toast, openPop, closePop, item, draggable };
})();
