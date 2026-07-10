import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useBaseUrl from '@docusaurus/useBaseUrl';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import HomepageFeatures from '@site/src/components/HomepageFeatures';

import Heading from '@theme/Heading';
import styles from './index.module.css';

// GitHub releases page — matches the download links in the repo README.
const RELEASES_URL = 'https://github.com/iffy/BearCAD/releases/latest';
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
        <p className={styles.heroBlurb}>
          BearCAD is a free, parametric CAD app for designing real parts —
          sketch, dimension, extrude, and export for 3D printing. The whole
          app is a single-digit-megabyte download that launches in about half
          a second — where mainstream CAD wants 8–16&nbsp;GB and a splash
          screen. Run it in your browser, or download it for macOS, Windows,
          or Linux. It's also an experiment to see what AI can do: the app is
          built almost entirely by an AI from plain-English requests.
        </p>
        <div className={styles.buttons}>
          <Link
            className={clsx('button button--lg', styles.ctaButton)}
            href={WEB_APP_PATH}>
            ▶&nbsp;&nbsp;Run in your browser
          </Link>
          <Link
            className="button button--outline button--secondary button--lg"
            href={RELEASES_URL}>
            Download
          </Link>
          <Link
            className="button button--outline button--secondary button--lg"
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
