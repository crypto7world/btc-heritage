use core::any::Any;

use btc_heritage_wallet::{
    errors::{Error, Result},
    heritage_service_api_client::{
        EmailAddress, HeirContact, HeirPermission, HeirPermissions, HeirUpdate,
        HeritageServiceClient, MainContact,
    },
};

/// Sub-command for heirs.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum HeirSubcmd {
    /// Get infos on the heir
    Get,
    /// Update the heir in the service
    Update {
        /// Change the name of the heir
        #[arg(long)]
        name: Option<String>,
        /// Change the main contact email address
        #[arg(long)]
        email: Option<String>,
        /// Change the custom message to include in communications with the heir
        #[arg(long)]
        custom_message: Option<String>,
        /// Remove the custom message if present
        #[arg(long, default_value_t = false, conflicts_with = "custom_message")]
        remove_custom_message: bool,
        /// Change the permissions of the heir
        #[arg(long, value_enum)]
        permissions: Option<Vec<CliHeirPermission>>,
    },
    /// Add contacts to the Heir
    AddContacts { contacts: Vec<String> },
    /// Remove contacts from the Heir
    RemoveContacts { contacts: Vec<String> },
}

impl super::CommandExecutor for HeirSubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let (heir_id, service_client): (String, HeritageServiceClient) =
            *params.downcast().unwrap();

        let res: Box<dyn crate::display::Displayable> = match self {
            HeirSubcmd::Get => Box::new(service_client.get_heir(&heir_id)?),
            HeirSubcmd::Update {
                name,
                email,
                custom_message,
                remove_custom_message,
                permissions,
            } => {
                let display_name = name;
                let custom_message_change = custom_message.is_some() || remove_custom_message;
                let email_change = email.is_some();
                let main_contact_change = custom_message_change || email_change;

                let main_contact = if main_contact_change {
                    let current_heir = service_client.get_heir(&heir_id)?;
                    Some(MainContact {
                        email: email
                            .map(|s| EmailAddress::try_from(s))
                            .unwrap_or(Ok(current_heir.main_contact.email))
                            .map_err(|e| Error::Generic(e))?,
                        custom_message: if remove_custom_message {
                            None
                        } else {
                            if custom_message.is_some() {
                                custom_message
                            } else {
                                current_heir.main_contact.custom_message
                            }
                        },
                    })
                } else {
                    None
                };

                let permissions = permissions
                    .map(|vhp| HeirPermissions::from(vhp.into_iter().map(|cli_hp| cli_hp.into())));

                let heir_update = HeirUpdate {
                    display_name,
                    main_contact,
                    permissions,
                };
                Box::new(service_client.patch_heir(&heir_id, heir_update)?)
            }
            HeirSubcmd::AddContacts { contacts } => Box::new(
                service_client.post_heir_contacts(
                    &heir_id,
                    contacts
                        .into_iter()
                        .map(|c| {
                            Ok(HeirContact::Email {
                                email: EmailAddress::try_from(c).map_err(|e| Error::Generic(e))?,
                            })
                        })
                        .collect::<Result<_>>()?,
                )?,
            ),
            HeirSubcmd::RemoveContacts { contacts } => Box::new(
                service_client.delete_heir_contacts(
                    &heir_id,
                    contacts
                        .into_iter()
                        .map(|c| {
                            Ok(HeirContact::Email {
                                email: EmailAddress::try_from(c).map_err(|e| Error::Generic(e))?,
                            })
                        })
                        .collect::<Result<_>>()?,
                )?,
            ),
        };
        Ok(res)
    }
}
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum CliHeirPermission {
    /// The Heir can see informations before maturity
    IsHeir,
    /// The Heir can see your email address
    OwnerEmail,
    /// The Heir can see the amount they may inherit
    Amount,
    /// The Heir can see the maturity dates of their inheritance
    Maturity,
    /// The heir can see their position in the HeritageConfigs and the total number of heirs
    Position,
}
impl From<CliHeirPermission> for HeirPermission {
    fn from(value: CliHeirPermission) -> Self {
        match value {
            CliHeirPermission::IsHeir => HeirPermission::IsHeir,
            CliHeirPermission::OwnerEmail => HeirPermission::OwnerEmail,
            CliHeirPermission::Amount => HeirPermission::Amount,
            CliHeirPermission::Maturity => HeirPermission::Maturity,
            CliHeirPermission::Position => HeirPermission::Position,
        }
    }
}
