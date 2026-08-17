import Link from '@docusaurus/Link';
import {DOWNLOADS, PAY_URL} from '@site/src/site';
import {DollarIcon, ICONS} from './icons';
import styles from './styles.module.css';

export default function GetBearCAD() {
  return (
    <div className={styles.wrap}>
      <div className={styles.pay}>
        <div className={styles.payCopy}>
          <p className={styles.payTitle}>Name your price</p>
          <p className={styles.payHint}>
            Pay what you want, or skip it. Paying helps further BearCAD's development.
          </p>
        </div>
        <Link className={styles.payButton} href={PAY_URL}>
          <DollarIcon />
          Pay Whatever
        </Link>
      </div>

      <div className={styles.downloads} id="downloads">
        {DOWNLOADS.map(({label, detail, href, icon}) => {
          const Icon = ICONS[icon];
          return (
            <Link key={label} className={styles.dl} href={href}>
              {Icon ? <Icon /> : null}
              <span className={styles.dlText}>
                <span className={styles.dlLabel}>{label}</span>
                <span className={styles.dlDetail}>{detail}</span>
              </span>
            </Link>
          );
        })}
      </div>
    </div>
  );
}
