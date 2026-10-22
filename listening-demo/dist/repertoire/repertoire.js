const player = document.querySelector('#chopin-audio');
const error = document.querySelector('#playback-error');
player.volume = .65;
player.addEventListener('error', () => { error.hidden = false; });
player.addEventListener('loadeddata', () => { error.hidden = true; });
