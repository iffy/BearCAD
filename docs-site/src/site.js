// Shared by the landing page and the Download doc.
export const PAY_URL = 'https://buy.stripe.com/4gMbJ39g2gsH4hKd9cdQQ00';
export const WEB_APP_PATH = 'pathname:///app/';

const RELEASE = 'https://github.com/iffy/BearCAD/releases/latest/download';

export const DOWNLOADS = [
  {
    label: 'macOS',
    detail: 'Apple Silicon',
    icon: 'macos',
    href: `${RELEASE}/bearcad.dmg`,
  },
  {
    label: 'Windows',
    detail: 'x86-64',
    icon: 'windows',
    href: `${RELEASE}/bearcad.exe`,
  },
  {
    label: 'Linux',
    detail: 'x86-64',
    icon: 'linux',
    href: `${RELEASE}/bearcad-linux-x86_64.tar.gz`,
  },
  {
    label: 'Browser',
    detail: 'No install',
    icon: 'browser',
    href: WEB_APP_PATH,
  },
];
