// Version selector for the docs site (issue #1231). Each published version
// lives at /<repo>/<version>/; /<repo>/versions.json lists them, newest first.
(function () {
  var parts = location.pathname.split("/");
  var root = "/" + parts[1] + "/";
  var current = parts[2] || "latest";
  fetch(root + "versions.json")
    .then(function (r) { return r.ok ? r.json() : ["latest"]; })
    .catch(function () { return ["latest"]; })
    .then(function (versions) {
      var bar = document.querySelector(".right-buttons");
      if (!bar) return;
      var sel = document.createElement("select");
      sel.setAttribute("aria-label", "Documentation version");
      versions.forEach(function (v) {
        var o = document.createElement("option");
        o.value = o.textContent = v;
        o.selected = v === current;
        sel.appendChild(o);
      });
      sel.onchange = function () {
        location.href = root + sel.value + "/" + parts.slice(3).join("/");
      };
      bar.prepend(sel);
    });
})();
