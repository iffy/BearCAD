// Shared by the landing page and the Download doc.
export const PAY_URL = 'https://buy.stripe.com/4gMbJ39g2gsH4hKd9cdQQ00';
export const WEB_APP_PATH = 'pathname:///app/';

const RELEASE = 'https://github.com/iffy/BearCAD/releases/latest/download';

export const DOWNLOADS = [
  {
    label: 'macOS',
    detail: 'Apple Silicon',
    href: `${RELEASE}/bearcad.dmg`,
  },
  {
    label: 'Windows',
    detail: 'x86-64',
    href: `${RELEASE}/bearcad.exe`,
  },
  {
    label: 'Linux',
    detail: 'x86-64',
    href: `${RELEASE}/bearcad-linux-x86_64.tar.gz`,
  },
  {
    label: 'Browser',
    detail: 'No install',
    href: WEB_APP_PATH,
  },
];
