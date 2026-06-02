(() => {
  function visibleText(n) {
    var c = n.cloneNode(true);
    c.querySelectorAll('style, script, template').forEach(function (x) { x.remove(); });
    return (c.textContent || '').replace(/\s+/g, ' ').trim();
  }
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
    elements.push({ selector: selector, tag: tag, attributes: attributes, text_content: txt, path: p, position_in_parent: pos });
  });
  return { html: document.documentElement.outerHTML.slice(0, 1000), elements: elements };
})()
