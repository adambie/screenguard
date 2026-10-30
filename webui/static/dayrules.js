// Day-based rules on the profile page — daily screen-time limits and allowed
// time windows. Both show only what is set, grouped by value ("Mon–Fri · 2h"),
// and share one editor: pick days (every day, weekdays, weekend or any days),
// enter the value, add. Saving writes the form's hidden fields and submits it.
window.SGDayRules = (() => {
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };
  const WEEK = [0, 1, 2, 3, 4, 5, 6], WEEKDAYS = [0, 1, 2, 3, 4], WEEKEND = [5, 6];
  const same = (a, b) => a.length === b.length && a.every((v, i) => v === b[i]);
  const hm = m => `${Math.floor(m / 60)}:${String(m % 60).padStart(2, '0')}`;
  const mounted = [];   // every editor on the page: saving one also saves the others' changes

  // minutes → "2h 30m"; "1:30" / "90" / "1h 30m" / "2h" / "45m" → minutes (null if empty, NaN if invalid)
  const dur = m => { const h = Math.floor(m / 60); return h ? (m % 60 ? `${h}h ${m % 60}m` : `${h}h`) : `${m}m`; };
  function parseDur(text) {
    const s = text.trim().toLowerCase().replace(/\s+/g, '');
    if (!s) return null;
    let m;
    if ((m = s.match(/^(\d{1,2}):([0-5]\d)$/))) return +m[1] * 60 + +m[2];
    if ((m = s.match(/^(\d{1,4})m?$/))) return +m[1];
    if ((m = s.match(/^(\d{1,2})h(?:([0-5]?\d)m?)?$/))) return +m[1] * 60 + (+m[2] || 0);
    return NaN;
  }

  // days → "Every day" / "Weekdays" / "Weekend" / "Mon–Thu" / "Mon, Wed, Sat"
  function daysLabel(days, T) {
    if (same(days, WEEK)) return T.every_day;
    if (same(days, WEEKDAYS)) return T.weekdays;
    if (same(days, WEEKEND)) return T.weekend;
    const runs = [];
    for (const d of days) {
      const last = runs[runs.length - 1];
      if (last && d === last[1] + 1) last[1] = d; else runs.push([d, d]);
    }
    return runs.map(([a, b]) => a === b ? T.days[a] : b === a + 1 ? `${T.days[a]}, ${T.days[b]}` : `${T.days[a]}–${T.days[b]}`).join(', ');
  }

  // `kind`: 'limit' (one per day: {dow, minutes}, 0 = blocked) or 'window'
  // (any number per day: {dow, start, end}). `T`: strings + day names.
  function mount(root, { kind, entries, T, form, write }) {
    const isLimit = kind === 'limit';
    const keyOf = e => isLimit ? String(e.minutes) : `${e.start}-${e.end}`;
    let list = entries.map(e => ({ ...e }));
    let editing = null;          // key of the group loaded into the editor
    let pick = [];               // days selected in the editor
    let saved = JSON.stringify(sorted(list));
    const dirty = () => JSON.stringify(sorted(list)) !== saved;

    function sorted(l) { return [...l].sort((a, b) => a.dow - b.dow || keyOf(a).localeCompare(keyOf(b))); }
    function groups() {
      const by = new Map();
      for (const e of sorted(list)) {
        if (!by.has(keyOf(e))) by.set(keyOf(e), { key: keyOf(e), sample: e, days: [] });
        by.get(keyOf(e)).days.push(e.dow);
      }
      return [...by.values()].sort((a, b) => a.days[0] - b.days[0] || a.key.localeCompare(b.key, undefined, { numeric: true }));
    }

    const listEl = el('div', 'sg-rlist');
    const editor = el('div', 'sg-redit');
    const foot = el('div', 'd-flex align-items-center justify-content-end gap-3 mt-3');
    const unsaved = el('span', 'small text-secondary', T.unsaved);
    const saveBtn = el('button', 'btn btn-primary', T.save);
    foot.append(unsaved, saveBtn);
    root.replaceChildren(listEl, editor, foot);

    function weekDots(days) {
      const w = el('div', 'sg-week');
      w.setAttribute('aria-hidden', 'true');
      WEEK.forEach(d => w.append(el('span', days.includes(d) ? 'on' : '', T.days[d].slice(0, 2))));
      return w;
    }

    function renderList() {
      listEl.replaceChildren();
      const gs = groups();
      if (!gs.length) { listEl.append(el('div', 'sg-rempty', T.empty)); return; }
      for (const g of gs) {
        const row = el('div', 'sg-rule' + (editing === g.key ? ' sg-editing' : ''));
        row.tabIndex = 0; row.setAttribute('role', 'button');
        const label = daysLabel(g.days, T);
        const value = isLimit ? (g.sample.minutes === 0 ? T.blocked : dur(g.sample.minutes)) : `${g.sample.start} – ${g.sample.end}`;
        row.setAttribute('aria-label', `${label}: ${value}. ${T.edit}`);
        const val = el('div', 'sg-rval' + (isLimit && g.sample.minutes === 0 ? ' sg-rblocked' : ''), value);
        const rm = el('button', 'sg-rx', '×');
        rm.type = 'button'; rm.title = T.remove; rm.setAttribute('aria-label', `${label}: ${T.remove}`);
        rm.onclick = e => { e.stopPropagation(); list = list.filter(x => keyOf(x) !== g.key); if (editing === g.key) reset(); render(); };
        row.append(weekDots(g.days), el('div', 'sg-rdays', label), val, rm);
        const open = () => load(g);
        row.onclick = open;
        row.onkeydown = e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); open(); } };
        listEl.append(row);
      }
    }

    // ── the editor ──────────────────────────────────────────────────────
    const presets = el('div', 'sg-presets');
    const dayBtns = el('div', 'sg-daybtns');
    const valueBox = el('div', 'sg-rvalue');
    const addBtn = el('button', 'btn btn-sm btn-primary', T.add);
    const cancelBtn = el('button', 'btn btn-sm btn-link text-secondary', T.cancel);
    const hint = el('div', 'sg-rhint', T.hint);
    addBtn.type = cancelBtn.type = 'button';
    const actions = el('div', 'd-flex gap-2 align-items-center');
    actions.append(addBtn, cancelBtn);
    const row2 = el('div', 'sg-rrow');
    row2.append(valueBox, actions);
    editor.append(el('div', 'sg-rtitle'), presets, dayBtns, row2, hint);

    for (const [label, days] of [[T.every_day, WEEK], [T.weekdays, WEEKDAYS], [T.weekend, WEEKEND]]) {
      const b = el('button', 'sg-preset', label);
      b.type = 'button'; b.dataset.days = days.join();
      b.onclick = () => { pick = [...days]; renderPick(); };
      presets.append(b);
    }
    WEEK.forEach(d => {
      const b = el('button', 'sg-day', T.days[d]);
      b.type = 'button'; b.dataset.dow = d;
      b.onclick = () => { pick = pick.includes(d) ? pick.filter(x => x !== d) : [...pick, d].sort(); renderPick(); };
      dayBtns.append(b);
    });

    let inA, inB, blockBtn, blocked = false;
    if (isLimit) {
      inA = el('input', 'form-control form-control-sm sg-rinput');
      inA.placeholder = T.placeholder; inA.inputMode = 'text'; inA.maxLength = 8; inA.setAttribute('aria-label', T.value);
      blockBtn = el('button', 'sg-preset sg-blockday', T.block);
      blockBtn.type = 'button';
      blockBtn.onclick = () => { blocked = !blocked; renderValue(); };
      valueBox.append(inA, el('span', 'text-secondary small', T.or), blockBtn);
    } else {
      inA = el('input', 'form-control form-control-sm'); inA.type = 'time'; inA.setAttribute('aria-label', T.from);
      inB = el('input', 'form-control form-control-sm'); inB.type = 'time'; inB.setAttribute('aria-label', T.to);
      valueBox.append(inA, el('span', 'text-secondary', '–'), inB);
    }
    [inA, inB].forEach(i => i && (i.onkeydown = e => { if (e.key === 'Enter') { e.preventDefault(); add(); } }));

    function renderPick() {
      dayBtns.querySelectorAll('.sg-day').forEach(b => b.classList.toggle('on', pick.includes(+b.dataset.dow)));
      dayBtns.querySelectorAll('.sg-day').forEach(b => b.setAttribute('aria-pressed', pick.includes(+b.dataset.dow)));
      presets.querySelectorAll('.sg-preset').forEach(b => b.classList.toggle('on', same(pick, b.dataset.days.split(',').map(Number))));
    }
    function renderValue() {
      if (!isLimit) return;
      blockBtn.classList.toggle('on', blocked);
      blockBtn.setAttribute('aria-pressed', blocked);
      inA.disabled = blocked;
      if (blocked) inA.value = '';
    }
    function renderEditor() {
      editor.querySelector('.sg-rtitle').textContent = editing === null ? T.add_title : T.edit_title;
      addBtn.textContent = editing === null ? T.add : T.update;
      cancelBtn.hidden = editing === null;
      renderPick(); renderValue();
    }

    function load(g) {
      editing = g.key; pick = [...g.days];
      if (isLimit) { blocked = g.sample.minutes === 0; inA.value = blocked ? '' : hm(g.sample.minutes); }
      else { inA.value = g.sample.start; inB.value = g.sample.end; }
      render();
      editor.scrollIntoView({ block: 'nearest' });
      (blocked ? blockBtn : inA).focus();
    }
    function reset() {
      editing = null; pick = []; blocked = false;
      inA.value = ''; if (inB) inB.value = '';
      [inA, inB].forEach(i => i?.classList.remove('is-invalid'));
    }
    cancelBtn.onclick = () => { reset(); render(); };

    function add() {
      [inA, inB].forEach(i => i?.classList.remove('is-invalid'));
      if (!pick.length) { dayBtns.classList.add('sg-shake'); setTimeout(() => dayBtns.classList.remove('sg-shake'), 400); return; }
      let make;
      if (isLimit) {
        const m = blocked ? 0 : parseDur(inA.value);
        if (m === null || Number.isNaN(m) || m > 1440) { inA.classList.add('is-invalid'); inA.focus(); return; }
        make = dow => ({ dow, minutes: m });
        // one limit per day: the new one replaces what those days had
        list = list.filter(e => (editing !== null && keyOf(e) === editing) ? false : !pick.includes(e.dow));
      } else {
        const a = inA.value, b = inB.value;
        if (!a) { inA.classList.add('is-invalid'); inA.focus(); return; }
        if (!b || b <= a) { inB.classList.add('is-invalid'); inB.focus(); return; }
        make = dow => ({ dow, start: a, end: b });
        if (editing !== null) list = list.filter(e => keyOf(e) !== editing);
        list = list.filter(e => !(pick.includes(e.dow) && e.start === a && e.end === b));   // no duplicates
      }
      list.push(...pick.map(make));
      reset(); render();
    }
    addBtn.onclick = add;

    function render() {
      renderList(); renderEditor();
      const d = dirty();
      unsaved.hidden = !d;
      saveBtn.disabled = !d;
    }

    function fill() {
      if (editing === null && pick.length && (inA.value || blocked)) add();   // something typed but not added yet
      form.querySelectorAll('input[type=hidden].sg-gen').forEach(i => i.remove());
      for (const [name, value] of write(sorted(list))) {
        const i = el('input'); i.type = 'hidden'; i.className = 'sg-gen'; i.name = name; i.value = value;
        form.append(i);
      }
      saved = JSON.stringify(sorted(list));
    }
    const me = {
      dirty,
      // in the background, for when another editor's Save reloads the page
      async saveQuietly() { fill(); await fetch(form.action, { method: 'POST', body: new FormData(form), credentials: 'same-origin' }); },
    };
    mounted.push(me);

    saveBtn.type = 'button';
    saveBtn.onclick = async () => {
      saveBtn.disabled = true;
      for (const other of mounted) if (other !== me && other.dirty()) await other.saveQuietly();
      fill();
      form.submit();
    };
    window.addEventListener('beforeunload', e => { if (dirty()) { e.preventDefault(); e.returnValue = ''; } });

    render();
  }

  return { mount, parseDur, dur };
})();
