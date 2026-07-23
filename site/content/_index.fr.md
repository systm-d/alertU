+++
title = "AlertU"

[extra]
nav_label = "Principal"
nav_docs = "Docs"
nav_download = "Télécharger"
langs_label = "Langue"

hero_eyebrow = "Comme une alarme de voiture — pour votre ordinateur"
hero_title = "Verrouillez. Éloignez-vous. AlertU monte la garde."
hero_lede = "Armez votre machine d'un clic : l'écran se verrouille, un bip retentit, et une veille silencieuse commence. Touchez-la en votre absence et AlertU déclenche l'alarme, prend une photo, et alerte votre téléphone."
cta_download = "Télécharger"
cta_github = "GitHub"
cta_docs = "Documentation"
hero_meta = "Libre et open source · Linux + systemd · MIT"

how_eyebrow = "Fonctionnement"
how_title = "Six étapes, un réflexe"
how_lede = "La boucle d'une alarme de voiture — armer, surveiller, réagir — au calme dans un démon sur votre machine."
step_arm = "Armer"
step_arm_d = "Un clic verrouille la session."
step_leave = "Partir"
step_leave_d = "Allez chercher un café. La veille démarre."
step_detect = "Détecter"
step_detect_d = "Une touche ou la souris la déclenche."
step_alarm = "Alarme"
step_alarm_d = "Un compte à rebours, puis la sirène."
step_snap = "Photo"
step_snap_d = "La webcam enregistre un cliché."
step_disarm = "Désarmer"
step_disarm_d = "Recliquez — retour au calme."

detect_eyebrow = "Intrusion"
detect_title = "Elle sait dès qu'on y touche"
detect_body = "Une fois armée, AlertU surveille vos périphériques d'entrée au niveau du noyau. Un mouvement de souris, une frappe au clavier, un effleurement du trackpad — toute activité sur un périphérique surveillé lance un compte à rebours réglable, avec un tic d'avertissement discret."
detect_t1 = "Surveille tout périphérique d'entrée — clavier, souris, trackpad"
detect_t2 = "Un délai de grâce après l'armement, pour partir tranquille"
detect_t3 = "X11 ou Wayland — elle ne parle qu'à logind"

alarm_eyebrow = "Alarme"
alarm_title = "Puis elle fait du bruit"
alarm_body = "Quand le compte à rebours expire, la sirène tourne en boucle et la machine cesse d'être discrète. Branchez le webhook optionnel sur votre téléphone, Slack ou Discord et l'alerte arrive dans votre poche, où que vous soyez."
alarm_t1 = "Une sirène en boucle, assez forte pour faire tourner les têtes"
alarm_t2 = "Un webhook optionnel — envoyez-le sur votre téléphone, à votre façon"

capture_eyebrow = "Preuve"
capture_title = "Et retient le visage"
capture_body = "À l'instant où l'alarme se déclenche, AlertU enregistre un cliché webcam horodaté. Retrouvez une petite galerie de qui s'est penché sur votre écran pendant votre absence."
capture_t1 = "Des clichés horodatés, enregistrés localement"
capture_t2 = "Gardés privés — mode 0640, jamais lisibles par tous"

disarm_eyebrow = "Désarmement"
disarm_title = "Désarmez comme vous avez armé"
disarm_body = "Votre télécommande est la clé. Un second clic, ou un déverrouillage normal par mot de passe, ramène tout au calme. Le premier qui arrive gagne — déverrouillez pendant le compte à rebours et rien ne se déclenche."
disarm_t1 = "N'importe quelle télécommande USB ou Bluetooth devient la clé"
disarm_t2 = "Ou déverrouillez simplement l'écran — logind prévient AlertU en temps réel"

dev_eyebrow = "Conçu pour les développeurs"
dev_title = "Petit, local, et franc là-dessus"
dev_lede = "Pas de cloud, pas de compte, pas de télémétrie. Quatre petits binaires qui dialoguent sur une seule socket locale."
card1_t = "Tout bureau Linux"
card1_d = "X11 et Wayland, toute distro avec systemd. Aucun environnement de bureau requis."
card2_t = "Open source"
card2_d = "Sous licence MIT, développé au grand jour. Lisez chaque ligne, envoyez un patch."
card3_t = "Vie privée d'abord"
card3_d = "Tout reste sur votre machine. La seule chose qui part est le webhook que vous configurez."
card4_t = "Installation rapide"
card4_d = "Un paquet pour votre distro, nommez votre télécommande, et vous êtes armé en une minute."
card5_t = "Léger"
card5_d = "Rust pur là où ça compte. Ni GTK, ni Qt, ni liaison ALSA."
card6_t = "Testé à fond"
card6_d = "102 tests à chaque push, avec rustfmt et clippy en -D warnings."

scen_eyebrow = "En pratique"
scen_title = "Une pause café, sous garde"
scen_lede = "Toute l'histoire, telle qu'elle se déroule vraiment."
scen1 = "Vous lancez une longue compilation."
scen2 = "Vous armez AlertU et vous levez."
scen3 = "Quelqu'un touche votre clavier."
scen4 = "La sirène se déclenche."
scen5 = "Un cliché est capturé."
scen6 = "Vous cliquez la télécommande — calme revenu."

install_eyebrow = "Installation"
install_title = "Un paquet pour votre distribution"
install_lede = "Chaque version publie un .rpm et un .deb — les quatre binaires, les unités systemd et les sons. Sur toute autre distro, compilez avec cargo."
install_source = "Source"
install_note = 'Un paquet ne peut ni vous ajouter à un groupe ni démarrer une unité utilisateur : quelques lignes restent à vous. Guide complet dans le <a href="https://github.com/systm-d/alertU#readme">README</a>.'

footer_quote_1 = "Toujours à veiller."
footer_quote_2 = "Jusqu'à votre retour."

ico_arm = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/></svg>'
ico_leave = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M13 3h6a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1h-6"/><path d="M10 12H3"/><path d="M6 8l-4 4 4 4"/></svg>'
ico_detect = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/></svg>'
ico_alarm = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M18 9a6 6 0 0 0-12 0c0 6-3 8-3 8h18s-3-2-3-8"/><path d="M10 20a2 2 0 0 0 4 0"/></svg>'
ico_snap = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 8h3l1.5-2h7L18 8h2a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1Z"/><circle cx="12" cy="13" r="3.5"/></svg>'
ico_disarm = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M8.5 12.5l2.5 2.5 4.5-5.5"/></svg>'
ico_touch = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M5 3l13 8-5.5 1.2L11 18z"/></svg>'
ico_coffee = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M5 9h11v5a4 4 0 0 1-4 4H9a4 4 0 0 1-4-4z"/><path d="M16 10h2a2 2 0 0 1 0 4h-2"/><path d="M8 3v2M11.5 3v2"/></svg>'
ico_platform = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/></svg>'
ico_open = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M8 6l-5 6 5 6"/><path d="M16 6l5 6-5 6"/></svg>'
ico_privacy = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l7 3v6c0 4-3 6.5-7 8-4-1.5-7-4-7-8V6z"/><path d="M9.5 12l2 2 3.5-4"/></svg>'
ico_fast = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M13 2 4 14h6l-1 8 9-12h-6z"/></svg>'
ico_light = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 20c0-8 6-14 16-15C19 13 13 20 4 20Z"/><path d="M9 15c2-2 5-3 8-4"/></svg>'
ico_secure = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/><circle cx="12" cy="15.5" r="1.2"/></svg>'
+++
