// Documentation remains usable without JavaScript.
if (navigator.clipboard && window.isSecureContext) {
  document.querySelectorAll('pre').forEach((pre) => {
    const code = pre.querySelector('code');
    if (!code) return;
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'copy-button';
    button.textContent = 'Copy';
    button.setAttribute('aria-label', 'Copy code block');
    button.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(code.textContent);
        button.textContent = 'Copied';
      } catch {
        button.textContent = 'Select to copy';
      }
      window.setTimeout(() => {
        button.textContent = 'Copy';
      }, 2000);
    });
    pre.prepend(button);
  });
}

// Interactive screenshot switcher on the overview page.
document.querySelectorAll('[data-showcase]').forEach((box) => {
  const tabs = [...box.querySelectorAll('.showcase-tab')];
  const img = box.querySelector('[data-showcase-img]');
  const titleEl = box.querySelector('[data-showcase-title]');
  const descEl = box.querySelector('[data-showcase-desc]');
  if (!tabs.length || !img) return;
  tabs.forEach((tab) => {
    tab.addEventListener('click', () => {
      tabs.forEach((t) => t.setAttribute('aria-selected', t === tab ? 'true' : 'false'));
      img.src = tab.getAttribute('data-src') || img.src;
      img.alt = tab.getAttribute('data-alt') || img.alt;
      if (titleEl) titleEl.textContent = tab.getAttribute('data-title') || '';
      if (descEl) descEl.textContent = tab.getAttribute('data-desc') || '';
    });
  });
});

// Click-to-zoom lightbox for screenshots.
const shotImages = [...document.querySelectorAll('.shot img')];
if (shotImages.length && typeof HTMLDialogElement !== 'undefined') {
  const dlg = document.createElement('dialog');
  dlg.className = 'lightbox';
  dlg.innerHTML =
    '<div class="lightbox-bar"><span id="lb-title"></span>' +
    '<button type="button" class="lightbox-close" aria-label="Close preview">Close (Esc)</button></div>' +
    '<img id="lb-img" alt="">';
  document.body.appendChild(dlg);
  const lbImg = dlg.querySelector('#lb-img');
  const lbTitle = dlg.querySelector('#lb-title');
  const closeBtn = dlg.querySelector('.lightbox-close');
  closeBtn?.addEventListener('click', () => dlg.close());
  dlg.addEventListener('click', (e) => {
    if (e.target === dlg) dlg.close();
  });
  shotImages.forEach((img) => {
    img.addEventListener('click', () => {
      lbImg.src = img.currentSrc || img.src;
      lbImg.alt = img.alt;
      lbTitle.textContent = img.getAttribute('data-caption') || img.alt;
      dlg.showModal();
    });
  });
}
