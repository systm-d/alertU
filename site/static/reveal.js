// AlertU — reveal-on-scroll. Elements marked `.reveal` fade and rise into
// place the first time they enter the viewport. If IntersectionObserver is
// missing, or the visitor prefers reduced motion, everything is shown at once.
(function () {
  "use strict";
  var nodes = document.querySelectorAll(".reveal");
  var reduce =
    window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  if (reduce || !("IntersectionObserver" in window)) {
    for (var i = 0; i < nodes.length; i++) nodes[i].classList.add("is-in");
    return;
  }

  var io = new IntersectionObserver(
    function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) {
          e.target.classList.add("is-in");
          io.unobserve(e.target);
        }
      });
    },
    { rootMargin: "0px 0px -12% 0px", threshold: 0.12 }
  );

  nodes.forEach(function (n) {
    io.observe(n);
  });
})();
