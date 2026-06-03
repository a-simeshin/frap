(() => {
  function visibleText(n) {
    var c = n.cloneNode(true);
    c.querySelectorAll('style, script, template').forEach(function (x) { x.remove(); });
    return (c.textContent || '').replace(/\s+/g, ' ').trim();
  }
  function norm(s) { return (s || '').replace(/\s+/g, ' ').trim(); }
  function structural(el) {
    var segs = []; var cur = el; var depth = 0;
    while (cur && cur !== document.body && depth < 5) {
      var tag = cur.tagName.toLowerCase();
      if (cur.id) { segs.unshift(tag + '#' + CSS.escape(cur.id)); break; }
      var tid = cur.getAttribute('data-testid');
      if (tid) { segs.unshift(tag + '[data-testid="' + tid + '"]'); break; }
      var parent = cur.parentElement;
      if (parent) {
        var node = cur;
        var same = Array.prototype.filter.call(parent.children, function (c) { return c.tagName === node.tagName; });
        var i = same.indexOf(cur);
        segs.unshift(same.length > 1 ? tag + ':nth-of-type(' + (i + 1) + ')' : tag);
      } else { segs.unshift(tag); }
      cur = cur.parentElement; depth++;
    }
    return segs.join(' > ');
  }
  function isVisible(el) {
    return el.getClientRects().length > 0 || el.offsetParent !== null;
  }
  var TEXT_INPUT = { text: 1, search: 1, email: 1, tel: 1, url: 1, password: 1, number: 1 };
  var BUTTON_INPUT = { submit: 1, button: 1, reset: 1 };
  function computedRole(el, tag) {
    var explicit = el.getAttribute('role');
    if (explicit) return explicit;
    if (tag === 'a') return 'link';
    if (tag === 'button') return 'button';
    if (tag === 'textarea') return 'textbox';
    if (tag === 'select') return 'combobox';
    if (tag === 'nav') return 'navigation';
    if (tag === 'input') {
      var t = (el.getAttribute('type') || 'text').toLowerCase();
      if (TEXT_INPUT[t]) return 'textbox';
      if (t === 'checkbox') return 'checkbox';
      if (t === 'radio') return 'radio';
      if (BUTTON_INPUT[t]) return 'button';
    }
    return null;
  }
  function labelledByText(el) {
    var ref = el.getAttribute('aria-labelledby');
    if (!ref) return '';
    var parts = [];
    ref.split(/\s+/).forEach(function (id) {
      if (!id) return;
      var t = document.getElementById(id);
      if (t) parts.push(t.textContent || '');
    });
    return norm(parts.join(' '));
  }
  function associatedLabel(el) {
    if (el.id) {
      var lbl = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
      if (lbl) return norm(lbl.textContent || '');
    }
    var cur = el.parentElement;
    while (cur) {
      if (cur.tagName === 'LABEL') return norm(cur.textContent || '');
      cur = cur.parentElement;
    }
    return '';
  }
  function accessibleName(el, tag) {
    var ariaLabel = norm(el.getAttribute('aria-label'));
    if (ariaLabel) return ariaLabel;
    var byLabelledby = labelledByText(el);
    if (byLabelledby) return byLabelledby;
    var byLabel = associatedLabel(el);
    if (byLabel) return byLabel;
    if (tag === 'img' || tag === 'input') {
      var alt = norm(el.getAttribute('alt'));
      if (alt) return alt;
    }
    var title = norm(el.getAttribute('title'));
    if (title) return title;
    var txt = visibleText(el).slice(0, 100);
    if (txt) return txt;
    return null;
  }
  var LANDMARKS = { HEADER: 1, FOOTER: 1, NAV: 1, MAIN: 1, ASIDE: 1, FORM: 1 };
  function scopeHint(el) {
    var cur = el.parentElement; var depth = 0;
    while (cur && depth < 25) {
      if (LANDMARKS[cur.tagName]) {
        var sel = cur.tagName.toLowerCase();
        if (cur.id) sel += '#' + CSS.escape(cur.id);
        var al = cur.getAttribute('aria-label');
        if (al) sel += '[aria-label="' + al + '"]';
        return sel;
      }
      cur = cur.parentElement; depth++;
    }
    return null;
  }
  var els = document.querySelectorAll('button, input, a, select, textarea, [data-testid], [data-id], li[id], [role="button"], [role="link"], [role="input"]');
  var elements = [];
  els.forEach(function (el) {
    var attributes = {};
    Array.prototype.forEach.call(el.attributes, function (a) { attributes[a.name] = a.value; });
    var p = []; var cur = el;
    while (cur && cur !== document.body) {
      p.unshift(cur.tagName.toLowerCase() + ':' + (cur.getAttribute('role') || '-'));
      cur = cur.parentElement;
    }
    var tag = el.tagName.toLowerCase();
    var testId = el.getAttribute('data-testid');
    var dataId = el.getAttribute('data-id');
    var selector;
    if (testId) selector = '[data-testid="' + testId + '"]';
    else if (dataId) selector = tag + '[data-id="' + dataId + '"]';
    else if (el.id) selector = tag + '[id="' + el.id + '"]';
    else selector = structural(el);
    var pos;
    var parent = el.parentElement;
    if (parent) {
      var sib = Array.prototype.filter.call(parent.children, function (c) { return c.tagName === el.tagName; });
      var idx = sib.indexOf(el);
      if (idx >= 0) pos = idx;
    }
    var txt = visibleText(el).slice(0, 100) || undefined;
    elements.push({
      selector: selector,
      tag: tag,
      attributes: attributes,
      text_content: txt,
      path: p,
      position_in_parent: pos,
      visible: isVisible(el),
      computed_role: computedRole(el, tag),
      accessible_name: accessibleName(el, tag),
      scope_hint: scopeHint(el)
    });
  });
  return { html: document.documentElement.outerHTML.slice(0, 1000), elements: elements };
})()
