import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useBaseUrl from '@docusaurus/useBaseUrl';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import HomepageFeatures from '@site/src/components/HomepageFeatures';

import Heading from '@theme/Heading';
import styles from './index.module.css';

// Latest-release asset URLs — same names as src/release_artifacts.rs and the repo README.
const RELEASES_BASE = 'https://github.com/iffy/BearCAD/releases/latest/download';
const DOWNLOADS = [
  {label: 'Download macOS', href: `${RELEASES_BASE}/bearcad.dmg`},
  {label: 'Download Windows', href: `${RELEASES_BASE}/bearcad.exe`},
  {label: 'Download Linux', href: `${RELEASES_BASE}/bearcad-linux-x86_64.tar.gz`},
];
// The hosted web build (wasm), deployed alongside the docs by CI.
const WEB_APP_PATH = 'pathname:///app/';

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={clsx('hero hero--primary', styles.heroBanner)}>
      <div className="container">
        <img
          className={styles.heroLogo}
          src={useBaseUrl('/img/logo.png')}
          alt="BearCAD bear icon"
          width="160"
          height="160"
        />
        <Heading as="h1" className="hero__title">
          {siteConfig.title}
        </Heading>
        <p className="hero__subtitle">{siteConfig.tagline}</p>
        <div className={styles.ctaRow}>
          <Link
            className={clsx('button button--lg', styles.ctaButton)}
            href={WEB_APP_PATH}>
            ▶&nbsp;&nbsp;Run in your browser
          </Link>
        </div>
        <div className={styles.buttons}>
          {DOWNLOADS.map(({label, href}) => (
            <Link
              key={label}
              className={clsx('button button--lg', styles.downloadButton)}
              href={href}>
              {label}
            </Link>
          ))}
          <Link
            className={clsx('button button--lg', styles.downloadButton)}
            to="/docs/intro">
            Read the docs
          </Link>
        </div>
        <span className={styles.ctaHint}>Nothing to install — it runs right in the tab.</span>
      </div>
    </header>
  );
}

function HomepageScreenshot() {
  return (
    <section className={styles.screenshotSection}>
      <div className="container">
        <img
          className={styles.screenshot}
          src={useBaseUrl('/img/screenshots/quickstart.png')}
          alt="BearCAD editing the Quickstart's 120-degree bracket: rounded bend, countersunk screw holes"
        />
        <p className={styles.screenshotCaption}>
          The <Link to="/docs/quickstart">Quickstart</Link> bracket — sketched freehand, squared
          up by the constraint solver, rebuilt from parameters.
        </p>
      </div>
    </section>
  );
}

export default function Home() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <Layout
      title={siteConfig.title}
      description="BearCAD — local-first, parametric CAD with a shared GUI and Lua scripting action layer.">
      <HomepageHeader />
      <main>
        <HomepageScreenshot />
        <HomepageFeatures />
      </main>
    </Layout>
  );
}
