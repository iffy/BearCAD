import Link from '@docusaurus/Link';
import {DOWNLOADS, PAY_URL} from '@site/src/site';
import styles from './styles.module.css';

export default function GetBearCAD() {
  return (
    <div className={styles.wrap}>
      <div className={styles.pay}>
        <div className={styles.payCopy}>
          <p className={styles.payTitle}>Name your price</p>
          <p className={styles.payHint}>
            BearCAD is free. Paying is optional.
          </p>
        </div>
        <Link className={styles.payButton} href={PAY_URL}>
          Name a price
        </Link>
      </div>

      <div className={styles.downloads} id="downloads">
        {DOWNLOADS.map(({label, detail, href}) => (
          <Link key={label} className={styles.dl} href={href}>
            <span className={styles.dlLabel}>{label}</span>
            <span className={styles.dlDetail}>{detail}</span>
          </Link>
        ))}
      </div>
    </div>
  );
}
