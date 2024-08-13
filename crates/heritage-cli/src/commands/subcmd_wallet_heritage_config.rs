use core::{any::Any, cell::RefCell};
use std::{collections::HashMap, rc::Rc};

use btc_heritage_wallet::{
    btc_heritage::{
        heritage_config::v1::{Days, Heritage},
        AccountXPub, HeirConfig, HeritageConfig, HeritageConfigVersion, SingleHeirPubkey,
    },
    errors::{Error, Result},
    AnyOnlineWallet, Database, DatabaseItem, Heir, OnlineWallet, Wallet,
};
use chrono::{NaiveDate, NaiveTime};

/// Wallet Heritage Configuration management subcommand.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletHeritageConfigSubcmd {
    /// List the existing Heritage Configurations of the wallet
    List,
    /// Display the current Heritage Configuration of the wallet
    ShowCurrent,
    /// Set a new Heritage Conguration of the wallet
    #[group(id = "heritage", multiple = true, required = false)]
    Set {
        /// The full Heritage Configuration as a JSON
        #[arg(long, value_parser=parse_heritage_configuration, conflicts_with="manual_spec", required_unless_present="manual_spec", help_heading="JSON specification")]
        json: Option<HeritageConfig>,
        #[command(flatten, next_help_heading = "Manual specification")]
        manual_spec: Option<ManualSpec>,
    },
}

#[derive(Debug, Clone, clap::Args)]
#[group(id = "manual_spec", conflicts_with = "json")]
pub struct ManualSpec {
    /// The version (only v1 exist for now)
    #[arg(short, long, default_value = "v1", requires_ifs=[("v1", "heritage")])]
    version: HeritageConfigVersion,
    /// The reference date from which every Heirs absolute locktime is computed. [default: today at noon]
    #[arg(short, long, value_name = "DATE")]
    reference_date: Option<NaiveDate>,
    /// The minimum lock time, in days. It enforces a minimum waiting period before any Heir can spend Bitcoins.
    /// {n}Usefull if an old address with an already passed absolute lock time receives coins. [default: 30 days]
    #[arg(short, long, value_name="DAYS", value_parser=parse_days)]
    minimum_lock_time: Option<Days>,
    /// An Heir, consisting of an Heir and a lock duration in days.
    /// {n}<LOCAL_HEIR> is a reference to a locally declared Heir (see the "heir" sub-command)
    /// {n}<LOCK_DAYS> is the number of days from the `reference_date` before this Heir can access the funds
    /// {n}Can be specified multiple times.
    #[arg(long, visible_alias="lh", value_name="LOCAL_HEIR>:<LOCK_DAYS", value_parser=parse_heir, group("heritage"))]
    local_heir: Vec<(String, Days)>,
    /// An Heir, consisting of an Heir and a lock duration in days.
    /// {n}<SERVICE_HEIR> is a reference to a service declared Heir (see the "service heir" sub-command)
    /// {n}<LOCK_DAYS> is the number of days from the `reference_date` before this Heir can access the funds
    /// {n}Can be specified multiple times.
    #[arg(long, visible_alias="sh", value_name="SERVICE_HEIR>:<LOCK_DAYS", value_parser=parse_heir, group("heritage"))]
    service_heir: Vec<(String, Days)>,
    /// An HeirConfig and a lock duration in days.
    /// {n}<KIND> is the kind of Heir Config (see the "heir" sub-command), can be ommited, defaults to xpub
    /// {n}<VALUE> is an appropriate string for the `KIND` of Heir Config
    /// {n}<LOCK_DAYS> is the number of days from the `reference_date` before this Heir can access the funds
    /// {n}Can be specified multiple times.
    #[arg(long, visible_alias="hc", value_name="KIND>:<VALUE>:<LOCK_DAYS", value_parser=parse_heir_config, group="heritage")]
    heir_config: Vec<(HeirConfig, Days)>,
}

impl super::CommandExecutor for WalletHeritageConfigSubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let (wallet, db): (Rc<RefCell<Wallet>>, Database) = *params.downcast().unwrap();
        let wallet = wallet.as_ref();
        let res: Box<dyn crate::display::Displayable> = match self {
            WalletHeritageConfigSubcmd::List => Box::new(wallet.borrow().list_heritage_configs()?),
            WalletHeritageConfigSubcmd::ShowCurrent => {
                Box::new(wallet.borrow().list_heritage_configs()?.remove(0))
            }
            WalletHeritageConfigSubcmd::Set { manual_spec, json } => {
                let hc = if let Some(manual_spec) = manual_spec {
                    let ManualSpec {
                        version,
                        reference_date,
                        minimum_lock_time,
                        local_heir,
                        service_heir,
                        heir_config,
                    } = manual_spec;
                    match version {
                        HeritageConfigVersion::V1 => {
                            let hcb = HeritageConfig::builder_v1();

                            let hcb = if let Some(minimum_lock_time) = minimum_lock_time {
                                hcb.minimum_lock_time(minimum_lock_time.as_u16())
                            } else {
                                hcb
                            };

                            let hcb = if let Some(reference_date) = reference_date {
                                let timestamp = reference_date
                                    .and_time(
                                        NaiveTime::from_num_seconds_from_midnight_opt(12 * 3600, 0)
                                            .unwrap(),
                                    )
                                    .and_utc()
                                    .timestamp();
                                hcb.reference_time(timestamp as u64)
                            } else {
                                hcb
                            };

                            let mut local_heirs_index: HashMap<String, HeirConfig> =
                                if local_heir.len() > 0 {
                                    Heir::all_in_db(&db)?
                                        .into_iter()
                                        .map(|h| (h.name, h.heir_config))
                                        .collect()
                                } else {
                                    Default::default()
                                };

                            let mut service_heirs_index: HashMap<String, HeirConfig> =
                                if service_heir.len() > 0 {
                                    if let AnyOnlineWallet::Service(sb) =
                                        wallet.borrow().online_wallet()
                                    {
                                        sb.service_client()
                                            .list_heirs()?
                                            .into_iter()
                                            .map(|h| (h.display_name, h.heir_config))
                                            .collect()
                                    } else {
                                        return Err(Error::IncorrectOnlineWallet("service"));
                                    }
                                } else {
                                    Default::default()
                                };

                            let hcb = hcb
                                .expand_heritages(
                                    local_heir
                                        .into_iter()
                                        .map(|(local_heir_name, time_lock)| {
                                            let heir_config = local_heirs_index
                                                .remove(&local_heir_name)
                                                .ok_or_else(|| {
                                                    Error::Generic(format!(
                                                        "{local_heir_name} \
                                                does not exist or was \
                                                specfied multiple times"
                                                    ))
                                                })?;
                                            Ok(Heritage {
                                                heir_config,
                                                time_lock,
                                            })
                                        })
                                        .collect::<Result<Vec<_>>>()?,
                                )
                                .expand_heritages(
                                    service_heir
                                        .into_iter()
                                        .map(|(service_heir_name, time_lock)| {
                                            let heir_config = service_heirs_index
                                                .remove(&service_heir_name)
                                                .ok_or_else(|| {
                                                    Error::Generic(format!(
                                                        "{service_heir_name} \
                                                does not exist or was \
                                                specified multiple times"
                                                    ))
                                                })?;
                                            Ok(Heritage {
                                                heir_config,
                                                time_lock,
                                            })
                                        })
                                        .collect::<Result<Vec<_>>>()?,
                                )
                                .expand_heritages(heir_config.into_iter().map(
                                    |(heir_config, time_lock)| Heritage {
                                        heir_config,
                                        time_lock,
                                    },
                                ));

                            hcb.build()
                        }
                    }
                } else if let Some(hc) = json {
                    hc
                } else {
                    unreachable!("either manual_spec or json must be present")
                };
                let new_hc = wallet.borrow_mut().set_heritage_config(hc)?;
                Box::new(new_hc)
            }
        };
        Ok(res)
    }
}

fn parse_heritage_configuration(
    val: &str,
) -> core::result::Result<HeritageConfig, serde_json::Error> {
    serde_json::from_str(val)
}

fn parse_days(val: &str) -> core::result::Result<Days, String> {
    Ok(val
        .parse::<Days>()
        .map_err(|e| format!("Could not parse as Days ({e})"))?)
}

fn parse_heir(val: &str) -> core::result::Result<(String, Days), String> {
    let splits = val.split(':').collect::<Vec<_>>();
    match splits.len() {
        2 => Ok((splits[0].to_owned(), parse_days(splits[1])?)),
        _ => Err(format!("Invalid number of parts: {}", splits.len())),
    }
}

fn parse_heir_config(val: &str) -> core::result::Result<(HeirConfig, Days), String> {
    let splits = val.split(':').collect::<Vec<_>>();
    let hc = match splits.len() {
        2 => HeirConfig::HeirXPubkey(AccountXPub::try_from(splits[0]).map_err(|e| e.to_string())?),
        3 => match splits[0] {
            "xpub" => HeirConfig::HeirXPubkey(
                AccountXPub::try_from(splits[1]).map_err(|e| e.to_string())?,
            ),
            "single-pub" => HeirConfig::SingleHeirPubkey(
                SingleHeirPubkey::try_from(splits[1]).map_err(|e| e.to_string())?,
            ),
            _ => return Err(format!("Invalid KIND: {}", splits[0])),
        },
        _ => return Err(format!("Invalid number of parts: {}", splits.len())),
    };
    Ok((hc, parse_days(splits[splits.len() - 1])?))
}
