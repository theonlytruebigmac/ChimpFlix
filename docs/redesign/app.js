/* ============================================================================
   ChimpFlix redesign mockups — shared shell + interactions.
   Each page sets <body data-context data-active data-crumbs>; this script
   injects the topbar, sidebar, context switcher, and command palette so the
   whole set behaves like one navigable prototype.
   ============================================================================ */
(function () {
  "use strict";

  /* ---- icon set (lucide-ish) -------------------------------------------- */
  var P = {
    user: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
    play: '<polygon points="6 4 20 12 6 20 6 4"/>',
    link: '<path d="M9 15l6-6"/><path d="M11 6l1-1a4 4 0 0 1 6 6l-1 1"/><path d="M13 18l-1 1a4 4 0 0 1-6-6l1-1"/>',
    bell: '<path d="M6 9a6 6 0 0 1 12 0c0 7 2 8 2 8H4s2-1 2-8"/><path d="M10 21a2 2 0 0 0 4 0"/>',
    monitor: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>',
    gauge: '<path d="M12 14l4-4"/><path d="M3.5 18a9 9 0 1 1 17 0z" fill="none"/>',
    film: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 4v16M17 4v16M3 9h4M17 9h4M3 15h4M17 15h4"/>',
    activity: '<path d="M3 12h4l3 8 4-16 3 8h4"/>',
    cpu: '<rect x="6" y="6" width="12" height="12" rx="2"/><path d="M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3"/>',
    users: '<circle cx="9" cy="8" r="3.5"/><path d="M3 20a6 6 0 0 1 12 0"/><path d="M16 5a3.5 3.5 0 0 1 0 7M21 20a6 6 0 0 0-5-5.9"/>',
    network: '<rect x="9" y="3" width="6" height="5" rx="1"/><rect x="3" y="16" width="6" height="5" rx="1"/><rect x="15" y="16" width="6" height="5" rx="1"/><path d="M12 8v4M6 16v-2a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v2"/>',
    mail: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M3 7l9 6 9-6"/>',
    key: '<circle cx="8" cy="14" r="4"/><path d="M11 11l9-9M17 5l2 2M14 8l2 2"/>',
    wrench: '<path d="M14 7a4 4 0 0 0-5 5l-6 6 2 2 6-6a4 4 0 0 0 5-5l-2 2-2-2 2-2z"/>',
    scroll: '<path d="M5 4h11v14a2 2 0 0 0 2 2H7a2 2 0 0 1-2-2z"/><path d="M16 4a2 2 0 0 1 2 2v2h-2"/><path d="M8 8h6M8 12h6"/>',
    settings: '<circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1l2-1.5-2-3.4-2.3 1a7 7 0 0 0-1.7-1l-.3-2.5h-4l-.3 2.5a7 7 0 0 0-1.7 1l-2.3-1-2 3.4 2 1.5a7 7 0 0 0 0 2l-2 1.5 2 3.4 2.3-1a7 7 0 0 0 1.7 1l.3 2.5h4l.3-2.5a7 7 0 0 0 1.7-1l2.3 1 2-3.4-2-1.5c.1-.3.1-.7.1-1z"/>',
    search: '<circle cx="11" cy="11" r="7"/><path d="M21 21l-4.3-4.3"/>',
    chevron: '<path d="M9 6l6 6-6 6"/>',
    chevrondown: '<path d="M6 9l6 6 6-6"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    check: '<path d="M5 12l5 5 9-11"/>',
    alert: '<path d="M12 3l9 16H3z"/><path d="M12 10v4M12 17v.5"/>',
    clock: '<circle cx="12" cy="12" r="8"/><path d="M12 8v4l3 2"/>',
    trash: '<path d="M4 7h16M9 7V5h6v2M6 7l1 13h10l1-13"/>',
    db: '<ellipse cx="12" cy="6" rx="8" ry="3"/><path d="M4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6"/><path d="M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3"/>',
    shield: '<path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"/>',
    folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
    layers: '<path d="M12 3l9 5-9 5-9-5z"/><path d="M3 13l9 5 9-5"/>',
    refresh: '<path d="M4 10a8 8 0 0 1 14-4l2 2M20 14a8 8 0 0 1-14 4l-2-2"/><path d="M18 4v4h-4M6 20v-4h4"/>',
    chip: '<rect x="7" y="7" width="10" height="10" rx="1"/><path d="M10 3v4M14 3v4M10 17v4M14 17v4M3 10h4M3 14h4M17 10h4M17 14h4"/>',
    sliders: '<path d="M4 8h10M18 8h2M4 16h2M10 16h10"/><circle cx="16" cy="8" r="2"/><circle cx="8" cy="16" r="2"/>',
    sparkles: '<path d="M12 4l1.5 4L18 9.5 13.5 11 12 15l-1.5-4L6 9.5 10.5 8z"/><path d="M18 15l.7 2 2 .7-2 .7-.7 2-.7-2-2-.7 2-.7z"/>',
    flow: '<rect x="3" y="4" width="6" height="4" rx="1"/><rect x="15" y="4" width="6" height="4" rx="1"/><rect x="9" y="16" width="6" height="4" rx="1"/><path d="M6 8v3a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V8M12 13v3"/>',
    grid: '<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>',
    home: '<path d="M4 11l8-7 8 7"/><path d="M6 10v9h12v-9"/>',
    eye: '<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/>'
  };
  function icon(name, cls) {
    var inner = P[name] || P.settings;
    return '<svg class="ico ' + (cls || '') + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' + inner + '</svg>';
  }

  /* ---- navigation model ------------------------------------------------- */
  var NAV = {
    personal: [
      { label: "Your account", items: [
        { key: "account", label: "Account", icon: "user", href: "account.html" },
        { key: "playback", label: "Playback", icon: "play", href: "playback.html" },
        { key: "integrations", label: "Integrations", icon: "link", href: "integrations.html" }
      ]},
      { label: "Preferences", items: [
        { key: "notifications", label: "Notifications", icon: "bell", href: "notifications.html" },
        { key: "devices", label: "Devices & sessions", icon: "monitor", href: "devices.html" },
        { key: "home", label: "Home & visibility", icon: "eye", href: "home-visibility.html" }
      ]}
    ],
    server: [
      { label: "Operate", items: [
        { key: "overview", label: "Overview", icon: "gauge", href: "admin-overview.html" },
        { key: "activity", label: "Activity & stats", icon: "activity", href: "admin-activity.html" }
      ]},
      { label: "Library", items: [
        { key: "libraries", label: "Libraries", icon: "film", href: "libraries.html" },
        { key: "tasks", label: "Tasks & jobs", icon: "flow", href: "admin-tasks.html", badge: "3" },
        { key: "transcoding", label: "Transcoding", icon: "cpu", href: "admin-transcoding.html" }
      ]},
      { label: "People", items: [
        { key: "users", label: "Users & access", icon: "users", href: "admin-users.html" }
      ]},
      { label: "System", items: [
        { key: "general", label: "General", icon: "settings", href: "admin-general.html" },
        { key: "network", label: "Network", icon: "network", href: "admin-network.html" },
        { key: "notif", label: "Notifications", icon: "mail", href: "admin-notifications.html" },
        { key: "credentials", label: "Credentials", icon: "key", href: "admin-credentials.html" },
        { key: "maintenance", label: "Maintenance", icon: "wrench", href: "admin-maintenance.html" },
        { key: "logs", label: "Logs & audit", icon: "scroll", href: "admin-logs.html" }
      ]}
    ]
  };

  /* ---- build shell ------------------------------------------------------ */
  function build() {
    var body = document.body;
    var ctx = body.getAttribute("data-context") || "personal";
    var active = body.getAttribute("data-active") || "";
    var crumbs = body.getAttribute("data-crumbs") || "";

    var main = document.querySelector("main.content");
    if (main) main.parentNode.removeChild(main);

    var shell = document.createElement("div");
    shell.className = "shell";

    /* topbar */
    var crumbHtml = crumbs.split("/").map(function (c, i, a) {
      c = c.trim();
      var last = i === a.length - 1;
      return (last ? "<b>" + c + "</b>" : c) + (last ? "" : ' <span class="sep">' + icon("chevron") + "</span> ");
    }).join("");
    shell.innerHTML =
      '<header class="topbar">' +
        '<a class="brand" href="index.html">CHIMPFLIX</a>' +
        '<div class="crumbs">' + crumbHtml + '</div>' +
        '<div class="topbar-spacer"></div>' +
        '<div class="cmdk" id="cmdkTrigger">' + icon("search") +
          '<span>Search settings &amp; actions</span><span class="kbd">⌘K</span></div>' +
        '<div class="topbar-avatar" title="Zach (Owner)">Z</div>' +
      '</header>' +
      '<div class="layout">' +
        '<aside class="sidebar">' + sidebar(ctx, active) + '</aside>' +
      '</div>';

    document.body.insertBefore(shell, document.body.firstChild);
    /* drop the main back in next to the sidebar */
    shell.querySelector(".layout").appendChild(main);

    buildPalette();
    wire();
  }

  function sidebar(ctx, active) {
    var sw =
      '<div class="ctx">' +
        '<button data-ctx="personal" class="' + (ctx === "personal" ? "on" : "") + '">' + icon("user") + 'You</button>' +
        '<button data-ctx="server" class="server ' + (ctx === "server" ? "on" : "") + '">' + icon("shield") + 'Server</button>' +
      '</div>';
    var groups = NAV[ctx].map(function (g) {
      var items = g.items.map(function (it) {
        if (it.soon) {
          return '<div class="nav-item soon">' + icon(it.icon) + '<span>' + it.label + '</span><span class="soon-tag">pattern</span></div>';
        }
        var badge = it.badge ? '<span class="nav-badge alert">' + it.badge + "</span>" : "";
        return '<a class="nav-item ' + (active === it.key ? "active" : "") + '" href="' + it.href + '">' +
          icon(it.icon) + "<span>" + it.label + "</span>" + badge + "</a>";
      }).join("");
      return '<div class="nav-group"><div class="nav-label">' + g.label + "</div>" + items + "</div>";
    }).join("");
    var foot = ctx === "server"
      ? '<div class="sidebar-foot">ChimpFlix v2.4.0 · press <span class="kbd">⌘K</span> to jump anywhere.</div>'
      : '<div class="sidebar-foot">Owner? Flip to <b>Server</b> above for the admin console.</div>';
    return sw + groups + foot;
  }

  /* ---- command palette -------------------------------------------------- */
  function buildPalette() {
    var rows = [];
    ["personal", "server"].forEach(function (ctx) {
      NAV[ctx].forEach(function (g) {
        g.items.forEach(function (it) {
          rows.push({ label: it.label, icon: it.icon, href: it.soon ? null : it.href,
            where: (ctx === "server" ? "Server" : "You") + " · " + g.label });
        });
      });
    });
    /* a few deep setting jumps to show "search any value" */
    [
      ["Change password", "shield", "account.html", "You · Account"],
      ["Two-factor (2FA)", "shield", "account.html", "You · Account"],
      ["Subtitle styling", "sliders", "playback.html", "You · Playback"],
      ["Autoplay next episode", "play", "playback.html", "You · Playback"],
      ["Link Trakt account", "link", "integrations.html", "You · Integrations"],
      ["Hardware acceleration", "cpu", "admin-transcoding.html", "Server · Transcoding"],
      ["Retry failed jobs", "refresh", "admin-tasks.html", "Server · Tasks"],
      ["Invite a user", "users", "admin-users.html", "Server · Users"],
      ["Library access matrix", "shield", "admin-users.html", "Server · Users"]
    ].forEach(function (d) { rows.push({ label: d[0], icon: d[1], href: d[2], where: d[3] }); });

    var ov = document.createElement("div");
    ov.className = "cmdk-overlay";
    ov.id = "cmdkOverlay";
    ov.innerHTML =
      '<div class="cmdk-panel" role="dialog">' +
        '<input class="cmdk-input" id="cmdkInput" placeholder="Jump to a page, setting, or action…" autocomplete="off">' +
        '<div class="cmdk-results" id="cmdkResults"></div>' +
        '<div class="cmdk-foot"><span><span class="kbd">↑↓</span> navigate</span><span><span class="kbd">↵</span> open</span><span><span class="kbd">esc</span> close</span></div>' +
      '</div>';
    document.body.appendChild(ov);

    function render(q) {
      q = (q || "").toLowerCase().trim();
      var list = rows.filter(function (r) { return !q || r.label.toLowerCase().indexOf(q) >= 0 || r.where.toLowerCase().indexOf(q) >= 0; });
      var res = document.getElementById("cmdkResults");
      if (!list.length) { res.innerHTML = '<div class="cmdk-row" style="cursor:default">No matches</div>'; return; }
      res.innerHTML = list.slice(0, 9).map(function (r, i) {
        return '<div class="cmdk-row ' + (i === 0 ? "sel" : "") + '" data-href="' + (r.href || "") + '">' +
          icon(r.icon) + "<span>" + r.label + '</span><span class="where">' + r.where + "</span></div>";
      }).join("");
      Array.prototype.forEach.call(res.querySelectorAll(".cmdk-row"), function (el) {
        el.addEventListener("click", function () { var h = el.getAttribute("data-href"); if (h) location.href = h; });
      });
    }
    ov._render = render;
    render("");
  }

  function openPalette() {
    var ov = document.getElementById("cmdkOverlay");
    ov.classList.add("open");
    var inp = document.getElementById("cmdkInput");
    inp.value = ""; ov._render(""); setTimeout(function () { inp.focus(); }, 30);
  }
  function closePalette() { document.getElementById("cmdkOverlay").classList.remove("open"); }

  /* ---- interactions ----------------------------------------------------- */
  function wire() {
    document.getElementById("cmdkTrigger").addEventListener("click", openPalette);
    document.getElementById("cmdkInput").addEventListener("input", function (e) {
      document.getElementById("cmdkOverlay")._render(e.target.value);
    });
    document.getElementById("cmdkOverlay").addEventListener("click", function (e) {
      if (e.target.id === "cmdkOverlay") closePalette();
    });
    document.addEventListener("keydown", function (e) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") { e.preventDefault(); openPalette(); }
      if (e.key === "Escape") closePalette();
      if (e.key === "Enter" && document.getElementById("cmdkOverlay").classList.contains("open")) {
        var sel = document.querySelector(".cmdk-row.sel"); if (sel) { var h = sel.getAttribute("data-href"); if (h) location.href = h; }
      }
    });

    /* context switcher */
    Array.prototype.forEach.call(document.querySelectorAll(".ctx button"), function (b) {
      b.addEventListener("click", function () {
        var c = b.getAttribute("data-ctx");
        location.href = c === "server" ? "admin-overview.html" : "account.html";
      });
    });

    /* tabs / mini-tabs (scoped to nearest [data-tabset]) */
    Array.prototype.forEach.call(document.querySelectorAll("[data-tab]"), function (t) {
      t.addEventListener("click", function () {
        var scope = t.closest("[data-tabset]") || document;
        var name = t.getAttribute("data-tab");
        // a "jump" link (not itself a tab) just triggers the matching real tab
        if (!t.classList.contains("tab") && !t.classList.contains("mtab")) {
          var real = scope.querySelector('.tab[data-tab="' + name + '"], .mtab[data-tab="' + name + '"]');
          if (real) { real.click(); return; }
        }
        var group = t.classList.contains("mtab") ? "mtab" : "tab";
        var owns = function (el) { return (el.closest("[data-tabset]") || document) === scope; };
        Array.prototype.forEach.call(scope.querySelectorAll("." + group + "[data-tab]"), function (x) { if (owns(x)) x.classList.remove("on"); });
        t.classList.add("on");
        Array.prototype.forEach.call(scope.querySelectorAll("[data-panel]"), function (p) {
          if (owns(p)) p.classList.toggle("on", p.getAttribute("data-panel") === name);
        });
      });
    });

    /* toggle switches */
    Array.prototype.forEach.call(document.querySelectorAll(".switch"), function (s) {
      s.addEventListener("click", function () { s.classList.toggle("on"); });
    });

    /* segmented controls + swatches (visual selection) */
    ["seg", "swatches"].forEach(function (grp) {
      Array.prototype.forEach.call(document.querySelectorAll("." + grp), function (g) {
        Array.prototype.forEach.call(g.children, function (btn) {
          btn.addEventListener("click", function () {
            Array.prototype.forEach.call(g.children, function (x) { x.classList.remove("on"); });
            btn.classList.add("on");
            if (g._onpick) g._onpick(btn);
          });
        });
      });
    });

    /* master-detail selection with light data-binding into the drawer */
    Array.prototype.forEach.call(document.querySelectorAll(".md-list"), function (list) {
      Array.prototype.forEach.call(list.querySelectorAll(".md-item"), function (it) {
        it.addEventListener("click", function () {
          Array.prototype.forEach.call(list.querySelectorAll(".md-item"), function (x) { x.classList.remove("active"); });
          it.classList.add("active");
          var data = it.dataset;
          Array.prototype.forEach.call(document.querySelectorAll("[data-bind]"), function (el) {
            var k = el.getAttribute("data-bind");
            if (data[k] !== undefined) el.textContent = data[k];
          });
        });
      });
    });
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", build);
  else build();
})();
