(() => {
  'use strict';

  const header = document.querySelector('[data-header]');
  const menu = document.querySelector('[data-menu]');
  const mobile = document.querySelector('[data-mobile]');
  const mobileLinks = mobile ? [...mobile.querySelectorAll('a')] : [];
  const background = [document.querySelector('main'), document.querySelector('footer')];
  const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
  const easeOut = 'cubic-bezier(0.23, 1, 0.32, 1)';

  const playMotion = (element, keyframes, options) => {
    element.getAnimations().forEach((animation) => animation.cancel());
    if (reducedMotion.matches) return;
    element.animate(keyframes, { easing: easeOut, ...options });
  };

  let headerScrolled;
  const setHeader = () => {
    const scrolled = scrollY > 12;
    if (headerScrolled === scrolled) return;
    headerScrolled = scrolled;
    header?.classList.toggle('scrolled', scrolled);
  };
  const setMenu = (open, moveFocus = true) => {
    if (!menu || !mobile) return;
    if ((menu.getAttribute('aria-expanded') === 'true') === open) return;
    menu.setAttribute('aria-expanded', String(open));
    menu.setAttribute('aria-label', open ? 'Close menu' : 'Open menu');
    mobile.hidden = !open;
    document.body.classList.toggle('menu-open', open);
    background.forEach((element) => {
      if (element) element.inert = open;
    });
    if (open && moveFocus) mobileLinks[0]?.focus();
  };

  setHeader();
  addEventListener('scroll', setHeader, { passive: true });
  menu?.addEventListener('click', () => setMenu(menu.getAttribute('aria-expanded') !== 'true'));
  mobileLinks.forEach((link) => link.addEventListener('click', () => setMenu(false, false)));
  addEventListener('resize', () => {
    if (innerWidth > 1020) setMenu(false, false);
  });
  addEventListener('keydown', (event) => {
    if (menu?.getAttribute('aria-expanded') !== 'true' || !mobile) return;
    if (event.key === 'Escape') {
      setMenu(false, false);
      menu.focus();
      return;
    }
    if (event.key !== 'Tab') return;
    const first = mobileLinks[0];
    const last = mobileLinks.at(-1);
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      menu.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      menu.focus();
    } else if (event.shiftKey && document.activeElement === menu) {
      event.preventDefault();
      last?.focus();
    }
  });

  const tabs = [...document.querySelectorAll('[data-tab]')];
  const panels = [...document.querySelectorAll('[data-panel]')];
  const activate = (name, moveFocus = false, animateChange = false) => {
    const previousIndex = tabs.findIndex((tab) => tab.getAttribute('aria-selected') === 'true');
    const nextIndex = tabs.findIndex((tab) => tab.dataset.tab === name);
    tabs.forEach((tab) => {
      const selected = tab.dataset.tab === name;
      tab.setAttribute('aria-selected', String(selected));
      tab.tabIndex = selected ? 0 : -1;
      if (selected && moveFocus) tab.focus();
    });
    panels.forEach((panel) => {
      panel.getAnimations().forEach((animation) => animation.cancel());
      panel.hidden = panel.dataset.panel !== name;
    });
    const selectedPanel = panels.find((panel) => panel.dataset.panel === name);
    if (animateChange && selectedPanel && previousIndex !== nextIndex) {
      const offset = nextIndex > previousIndex ? '8px' : '-8px';
      playMotion(selectedPanel, [
        { opacity: 0.45, transform: `translateX(${offset})` },
        { opacity: 1, transform: 'translateX(0)' },
      ], { duration: 180 });
    }
  };
  tabs.forEach((tab, index) => {
    const panel = panels.find((candidate) => candidate.dataset.panel === tab.dataset.tab);
    if (panel) panel.setAttribute('aria-labelledby', tab.id ||= `terminal-tab-${index + 1}`);
    tab.addEventListener('click', (event) => activate(tab.dataset.tab, false, event.detail > 0));
    tab.addEventListener('keydown', (event) => {
      if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      let next = index;
      if (event.key === 'ArrowRight') next = (index + 1) % tabs.length;
      if (event.key === 'ArrowLeft') next = (index - 1 + tabs.length) % tabs.length;
      if (event.key === 'Home') next = 0;
      if (event.key === 'End') next = tabs.length - 1;
      activate(tabs[next].dataset.tab, true);
    });
  });

  const copyFallback = (value) => {
    const input = document.createElement('textarea');
    input.value = value;
    input.style.position = 'fixed';
    input.style.opacity = '0';
    document.body.appendChild(input);
    input.select();
    const copied = document.execCommand('copy');
    input.remove();
    return copied;
  };
  document.querySelectorAll('[data-copy]').forEach((button) => {
    const status = button.querySelector('span');
    const idleLabel = status.textContent;
    let resetTimer;

    button.addEventListener('click', async () => {
      let copied = false;
      try {
        await navigator.clipboard.writeText(button.dataset.copy);
        copied = true;
      } catch {
        copied = copyFallback(button.dataset.copy);
      }
      status.textContent = copied ? 'Copied' : 'Select command';
      button.classList.toggle('copied', copied);
      clearTimeout(resetTimer);
      resetTimer = setTimeout(() => {
        status.textContent = idleLabel;
        button.classList.remove('copied');
      }, 1700);
    });
  });

  document.querySelectorAll('[data-accordion]').forEach((root, rootIndex) => {
    const isFaq = root.classList.contains('faq-list');
    const items = [...root.children].filter((child) => child.matches('article'));
    const pairs = items.map((item, itemIndex) => {
      const trigger = item.querySelector(':scope > button');
      const body = trigger?.nextElementSibling;
      if (!trigger || !body) return null;
      const bodyId = `accordion-${rootIndex + 1}-panel-${itemIndex + 1}`;
      body.id = bodyId;
      trigger.setAttribute('aria-controls', bodyId);
      return { trigger, body };
    }).filter(Boolean);
    let openPair = pairs.find(({ trigger, body }) => trigger.getAttribute('aria-expanded') === 'true' && !body.hidden);

    pairs.forEach((pair) => {
      const { trigger, body } = pair;
      trigger.addEventListener('click', (event) => {
        const opening = trigger.getAttribute('aria-expanded') !== 'true';
        if (openPair) {
          openPair.body.getAnimations().forEach((animation) => animation.cancel());
          openPair.trigger.setAttribute('aria-expanded', 'false');
          openPair.body.hidden = true;
          openPair = undefined;
        }
        if (opening) {
          trigger.setAttribute('aria-expanded', 'true');
          body.hidden = false;
          openPair = pair;
          if (event.detail > 0) {
            const keyframes = isFaq ? [
              { clipPath: 'inset(0 0 100% 0)', opacity: 0, transform: 'translateY(-8px)' },
              { clipPath: 'inset(0)', opacity: 1, transform: 'translateY(0)' },
            ] : [
              { opacity: 0, transform: 'translateY(-5px)' },
              { opacity: 1, transform: 'translateY(0)' },
            ];
            playMotion(body, keyframes, { duration: isFaq ? 220 : 180 });

            const disclosure = isFaq ? trigger.querySelector('i') : null;
            if (disclosure) {
              playMotion(disclosure, [
                { transform: 'rotate(-45deg) scale(.72)' },
                { transform: 'rotate(0) scale(1)' },
              ], { duration: 200 });
            }
          }
        }
      });
    });
  });

  const motionGroups = [
    { root: '.proof__grid', items: ':scope > div', kind: 'grid' },
    { root: '.what__grid', items: ':scope > *', kind: 'split' },
    { root: '.surface__grid', items: ':scope > article', kind: 'grid' },
    { root: '.feature-grid', items: ':scope > article', kind: 'grid' },
    { root: '.workflow__grid', items: ':scope > *', kind: 'split' },
    { root: '.interface-grid', items: ':scope > article', kind: 'grid' },
    { root: '.command-grid', items: ':scope > article', kind: 'grid' },
    { root: '.diagram', items: ':scope > .node, :scope > .adapters', kind: 'flow' },
    { root: '.privacy__grid', items: ':scope > *', kind: 'split' },
    { root: '.faq-list', items: ':scope > article', kind: 'list' },
    { root: '.final__card', items: ':scope > *', kind: 'split' },
    { root: '.footer-links', items: ':scope > div', kind: 'grid' },
  ];

  const startingTransform = (kind, index) => {
    if (kind === 'split') return `translateX(${index === 0 ? '-18px' : '18px'}) scale(.985)`;
    if (kind === 'flow') {
      if (index === 0) return 'translateX(-20px)';
      if (index === 1) return 'translateY(14px) scale(.97)';
      return 'translateX(20px)';
    }
    if (kind === 'list') return 'translateX(-10px)';
    return 'translateY(14px) rotateX(3deg)';
  };

  if ('IntersectionObserver' in window && !reducedMotion.matches) {
    const configurations = new Map();
    const observer = new IntersectionObserver((entries) => entries.forEach((entry) => {
      if (!entry.isIntersecting) return;
      const configuration = configurations.get(entry.target);
      configuration.items.forEach((element, index) => playMotion(element, [
        { opacity: 0, transform: startingTransform(configuration.kind, index) },
        { opacity: 1, transform: 'none' },
      ], { duration: 420, delay: index * 35, fill: 'backwards' }));
      configurations.delete(entry.target);
      observer.unobserve(entry.target);
      if (configurations.size === 0) observer.disconnect();
    }), { threshold: 0.08, rootMargin: '0px 0px -8%' });

    motionGroups.forEach(({ root, items, kind }) => {
      const container = document.querySelector(root);
      if (!container) return;
      configurations.set(container, { items: [...container.querySelectorAll(items)], kind });
      observer.observe(container);
    });
  }

  document.querySelectorAll('[data-year]').forEach((element) => {
    element.textContent = new Date().getFullYear();
  });
})();
