+++
title = "AlertU"

[extra]
eyebrow = "Linux · logind · evdev"
tagline = "Verrouillez votre machine. Gardez le contrôle."
lede = "Une télécommande USB ou Bluetooth bon marché devient la clé de votre bureau Linux. Un clic pour armer : la session se verrouille et un bip retentit. Touchez la machine pendant qu'elle est armée et un compte à rebours démarre — puis une sirène, une photo webcam horodatée, et un webhook optionnel."
cta = "Voir sur GitHub"
cta2 = "Installer"
logo_alt = "Le logo AlertU : un bouclier néon en dégradé vert-cyan, une cloche d'alarme rouge à l'intérieur, des ondes rouges au-dessus."
langs_label = "Langue"
term_toggle = "exactement ce que fait un clic sur la télécommande"
+++

<section>
<p class="eyebrow">Fonctionnement</p>

## Quatre états, une seule télécommande

<p class="section__lede">Une seule tâche du démon détient tout l'état mutable et pilote chaque transition à partir de quatre sources multiplexées : les signaux d'entrée, les changements de verrouillage de session, les commandes IPC et les minuteries internes.</p>

<ol class="flow">
  <li class="s1"><p class="st"><span class="dot"></span>Idle</p><p>Désarmé. Rien n'est surveillé.</p></li>
  <li class="s2"><p class="st"><span class="dot"></span>Armed</p><p>Un clic sur la télécommande verrouille la session via <code>loginctl lock-session</code> et joue un bref bip. Les entrées surveillées deviennent actives après un délai de grâce.</p></li>
  <li class="s3"><p class="st"><span class="dot"></span>Triggered</p><p>Une activité sur un périphérique surveillé lance un compte à rebours réglable, accompagné d'un tic d'avertissement discret.</p></li>
  <li class="s4"><p class="st"><span class="dot"></span>Alarm</p><p>Le compte à rebours a expiré. La sirène tourne en boucle, une photo webcam horodatée est enregistrée, et le webhook optionnel est déclenché.</p></li>
</ol>

<p class="flow-note"><strong>Désarmement : le premier qui arrive gagne.</strong> Un nouveau clic sur la télécommande, ou un déverrouillage normal par mot de passe — lu en temps réel dans le <code>LockedHint</code> de logind via D-Bus, avec repli sur un sondage de <code>loginctl</code> si le bus est indisponible. Déverrouillez pendant le compte à rebours et tout revient à Idle.</p>
</section>

<hr class="divider" />
<section>
<p class="eyebrow">Ce que c'est</p>

## Petit, local, et franc là-dessus

<div class="feats">
  <div class="feat"><h3><span class="mark">◆</span> N'importe quelle télécommande</h3><p>Aucun modèle n'est codé en dur. Tout périphérique USB ou Bluetooth qui apparaît comme un nœud HID sous <code>/dev/input/eventX</code> fonctionne — un pointeur de présentation, un déclencheur photo Bluetooth, un clavier d'appoint — et <code>toggle_keys</code> accepte n'importe quel nom de touche evdev.</p></div>
  <div class="feat"><h3><span class="mark">◆</span> X11 et Wayland</h3><p>Il ne parle qu'à logind : aucune dépendance à un compositeur ou à un environnement de bureau. Linux avec systemd, c'est toute l'exigence.</p></div>
  <div class="feat"><h3><span class="mark">◆</span> Quatre binaires</h3><p>Un démon privilégié, une icône de barre d'état StatusNotifierItem, une fenêtre de réglages egui autonome, et <code>alertu-ctl</code>. Chaque interface parle au démon via une seule socket Unix locale, en JSON ligne à ligne.</p></div>
  <div class="feat"><h3><span class="mark">◆</span> 100 % local</h3><p>Pas de cloud, pas de télémétrie, pas de compte. La seule chose qui quitte un jour la machine est le webhook que vous configurez vous-même — et il est vide par défaut.</p></div>
  <div class="feat"><h3><span class="mark">◆</span> Rust pur là où ça compte</h3><p>La barre d'état utilise <code>zbus</code>, pas <code>libdbus</code> ; ni GTK, ni Qt, ni liaison ALSA. L'audio, la capture webcam et le webhook délèguent à <code>paplay</code>/<code>ffplay</code>, <code>fswebcam</code>/<code>ffmpeg</code> et <code>curl</code>.</p></div>
  <div class="feat"><h3><span class="mark">◆</span> Testé, et sous MIT</h3><p>102 tests — unitaires, plus des tests d'intégration qui pilotent un vrai démon par sa socket avec un faux <code>loginctl</code> — exécutés à chaque push, avec <code>rustfmt</code> et clippy en <code>-D warnings</code>.</p></div>
</div>
</section>

<hr class="divider" />
<section>
<p class="eyebrow">Votre télécommande</p>

## Aucune télécommande par défaut — délibérément

<p class="section__lede"><code>remote_name_hint</code> est vide au départ, et vide signifie <em>aucune télécommande</em>, pas « prends la première ». Une sous-chaîne vide correspondrait à tous les périphériques ; AlertU ne résout donc rien plutôt que de lier silencieusement votre bouton au nœud qui s'est énuméré en premier. Tant que vous n'avez pas nommé un périphérique, le démon le signale dans son journal et le bouton est simplement indisponible — tout le reste fonctionne.</p>

<div class="term">
<div class="term__bar" aria-hidden="true"><span class="term__dot"></span><span class="term__dot"></span><span class="term__dot"></span><span class="term__title">trouver sa télécommande</span></div>
<pre><code><span class="p">$</span> alertu-ctl list-devices
/dev/input/event3  AT Translated Set 2 keyboard [keyboard]
/dev/input/event5  Logitech USB Receiver [pointer]
/dev/input/event9  BT Camera Shutter [keyboard]

<span class="p">$</span> sudo journalctl -u alertu-daemon -f   <span class="dim"># puis pressez un bouton, en RUST_LOG=debug</span></code></pre>
</div>

<p class="flow-note">Il ne reste qu'à poser <code>remote_name_hint = "shutter"</code> et <code>toggle_keys = ["KEY_VOLUMEUP"]</code> — depuis la barre d'état, depuis la fenêtre de réglages, ou avec <code>alertu-ctl set-config</code>. Les périphériques surveillés valent <code>["auto"]</code> par défaut : tout sauf la télécommande et la souris principale.</p>
</section>

<hr class="divider" />
<section>
<p class="eyebrow">Ligne de commande</p>

## Tout ce que fait la barre d'état, depuis un script

<ul class="commands">
  <li><code>alertu-ctl status</code><span>Idle, Armed, Triggered ou Alarm</span></li>
  <li><code>alertu-ctl status --watch</code><span>une ligne par changement d'état, jusqu'à interruption</span></li>
  <li><code>alertu-ctl arm</code><span>armement forcé — verrouille la session</span></li>
  <li><code>alertu-ctl disarm</code><span>désarmement forcé — la déverrouille</span></li>
  <li><code>alertu-ctl toggle</code><span>exactement ce que fait un clic sur la télécommande</span></li>
  <li><code>alertu-ctl get-config</code><span>la configuration effective du démon, en TOML</span></li>
  <li><code>alertu-ctl set-config c.toml</code><span>la remplace (<code>-</code> lit stdin), validée localement d'abord</span></li>
  <li><code>alertu-ctl list-devices</code><span>les périphériques d'entrée que le démon voit</span></li>
  <li><code>alertu-ctl gen-sounds --dir …</code><span>écrit les trois fichiers son par défaut</span></li>
</ul>

<p class="flow-note"><code>--json</code> affiche la réponse brute du protocole du démon : une transition observée arrive donc en <code>{"event":"state_changed","state":"armed"}</code> et reste distinguable de l'instantané <code>state</code> initial. Codes de sortie : <code>0</code> succès, <code>1</code> erreur de démon ou de connexion, <code>2</code> erreur d'usage.</p>
</section>

<hr class="divider" />
<section>
<p class="eyebrow">Périmètre</p>

## Un gadget personnel, pas un système antivol

<div class="callout callout--alarm">
<p>Ce sont les mots du projet lui-même, et ce site ne prétendra pas le contraire. Il n'y a aucune protection anti-altération des binaires. La socket de contrôle est en <code>0660</code> dans le groupe du démon, et s'y connecter équivaut au contrôle total de l'alarme — la désarmer, lire la configuration y compris l'URL du webhook, et <code>SetConfig</code>, qui oriente les chemins passés aux programmes auxiliaires. Considérez l'appartenance au groupe comme un privilège accordé, pas comme un confort.</p>
<p>Les photos d'alarme relèvent de la même frontière : chaque cliché est écrit en <code>0640</code> dans ce groupe, dans un <code>snapshot_dir</code> que le démon maintient en <code>0750</code> quand il en est propriétaire. Délibérément non lisible par tous — une photo webcam de quiconque se trouve devant la machine, propriétaire compris, n'a rien à y faire. Un répertoire dont le démon n'est pas propriétaire est laissé tel quel, avec un avertissement, plutôt que re-permissionné.</p>
</div>
</section>

<hr class="divider" />
<section id="install">
<p class="eyebrow">Installation</p>

## Compilé depuis les sources, aujourd'hui

<p class="section__lede">Il n'existe pas encore de paquet publié — un paquet Debian est en cours. Pour l'instant, compilez et installez les unités systemd fournies. La marche à suivre complète, avec l'unité utilisateur de la barre d'état, les icônes et l'entrée de menu, est dans le <a href="https://github.com/systm-d/alertU#install-systemd">README</a>.</p>

<div class="steps">
  <div class="step">
    <h3>1 · Compiler</h3>

```sh
git clone https://github.com/systm-d/alertU
cd alertU
cargo build --release
```

<p>Nécessite une toolchain Rust stable récente. Seul <code>alertu-settings</code> tire des dépendances système (egui se lie à X11/Wayland/GL) — écartez-le avec <code>--workspace --exclude alertu-settings</code>.</p>
  </div>
  <div class="step">
    <h3>2 · Compte de service &amp; unités</h3>

```sh
sudo systemd-sysusers packaging/sysusers.d/alertu.conf
sudo install -Dm644 packaging/alertu-daemon.service \
  /etc/systemd/system/alertu-daemon.service
sudo alertu-ctl gen-sounds --dir /usr/share/sounds/alertu
sudo systemctl enable --now alertu-daemon
```

<p>C'est le compte du démon qui a besoin de <code>input</code> et <code>video</code> ; le vôtre n'a besoin d'aucun des deux.</p>
  </div>
  <div class="step">
    <h3>3 · Rejoindre le groupe de la socket</h3>

```sh
sudo usermod -aG alertu "$USER"
# puis ouvrez une nouvelle session, ou : newgrp alertu
```

<p>La socket est en <code>0660</code>. Sans ce groupe, la barre d'état, la fenêtre de réglages et <code>alertu-ctl</code> échouent tous à se connecter.</p>
  </div>
  <div class="step">
    <h3>4 · Programmes auxiliaires</h3>

```sh
# photos : l'un de
fswebcam  |  ffmpeg
# audio : l'un de
paplay  |  pw-play  |  aplay  |  ffplay  |  play
```

<p>AlertU délègue à ces outils au lieu de s'y lier : prenez celui que votre distribution embarque déjà.</p>
  </div>
</div>

<p class="callout"><strong>Une relecture Linux du vieil iAlertU du Mac.</strong> Sous licence MIT, développé au grand jour sur <a href="https://github.com/systm-d/alertU">systm-d/alertU</a>. La configuration vit dans <code>/etc/alertu/config.toml</code>, et chaque champ y est documenté en ligne dans <a href="https://github.com/systm-d/alertU/blob/main/packaging/config.example.toml"><code>packaging/config.example.toml</code></a>.</p>
</section>
