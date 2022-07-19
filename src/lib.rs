use scrypto::prelude::*;

#[derive(NonFungibleData)]
struct BankerTicketData {
    #[scrypto(mutable)]
    bank_amount: Decimal,
    #[scrypto(mutable)]
    borrow_amount: Decimal,
    deposit_epoch: u64,
}
impl BankerTicketData {
    pub fn max_borrow_amount(&self) -> Decimal {
        (self.bank_amount - self.borrow_amount) / dec!("1.5")
    }
    pub fn percentage_of_unused_collateral(&self) -> Decimal {
        dec!("1") - (self.borrow_amount / (self.bank_amount / dec!("1.5")))
    }
}

blueprint! {
    struct BankersParty {
        bank_pool: Vault,
        bankers_rewards: HashMap<NonFungibleId, Vault>,
        bankers_auth_badge: Vault,
        banker_ticket_address: ResourceAddress,
    }

    impl BankersParty {
        pub fn instantiate_bankers_party(
            token_resource_address: ResourceAddress,
        ) -> ComponentAddress {
            let banker_auth_badge: Bucket = ResourceBuilder::new_fungible()
                .divisibility(DIVISIBILITY_NONE)
                .metadata("name", "bank auth badge")
                .initial_supply(Decimal::one());

            let banker_ticket_address: ResourceAddress = ResourceBuilder::new_non_fungible()
                .metadata("name", "Banker Ticket")
                .mintable(rule!(require(banker_auth_badge.resource_address())), LOCKED)
                .burnable(rule!(require(banker_auth_badge.resource_address())), LOCKED)
                .updateable_non_fungible_data(
                    rule!(require(banker_auth_badge.resource_address())),
                    LOCKED,
                )
                .no_initial_supply();

            let component_address = Self {
                bank_pool: Vault::new(token_resource_address),
                bankers_rewards: HashMap::new(),
                banker_ticket_address: banker_ticket_address,
                bankers_auth_badge: Vault::with_bucket(banker_auth_badge),
            }
            .instantiate()
            .globalize();

            component_address
        }

        pub fn bank(&mut self, bank: Bucket) -> Bucket {
            //stake cannot be less then 100 xrd
            assert!(
                bank.amount() > dec!("100"),
                " Your Bank must be greater than 100 tokens"
            );

            let banker_ticket_id = NonFungibleId::random();

            let banker_ticket: Bucket = self.bankers_auth_badge.authorize(|| {
                borrow_resource_manager!(self.banker_ticket_address).mint_non_fungible(
                    &banker_ticket_id,
                    BankerTicketData {
                        bank_amount: bank.amount(),
                        borrow_amount: dec!("0"),
                        deposit_epoch: Runtime::current_epoch(),
                    },
                )
            });
            self.bankers_rewards.insert(
                banker_ticket_id,
                Vault::new(self.bank_pool.resource_address()),
            );

            self.bank_pool.put(bank);

            banker_ticket
        }

        pub fn unbank(&mut self, ticket: Bucket) -> Bucket {
            assert!(ticket.resource_address() == self.banker_ticket_address);

            let data: BankerTicketData = ticket.non_fungible().data();

            assert!(Runtime::current_epoch() >= data.deposit_epoch + 500);
            assert!(data.borrow_amount == dec!("0"));

            let mut returned_bank: Bucket = self.bank_pool.take(data.bank_amount);

            let returned_rewards: Bucket = self
                .bankers_rewards
                .get_mut(&ticket.non_fungible::<BankerTicketData>().id())
                .unwrap()
                .take_all();

            let resource_manager: &ResourceManager =
                borrow_resource_manager!(self.banker_ticket_address);

            self.bankers_auth_badge
                .authorize(|| resource_manager.burn(ticket));

            returned_bank.put(returned_rewards);
            returned_bank
        }

        pub fn reduce_bank(&mut self, ticket: Proof, amount: Decimal) -> Bucket {
            assert!(ticket.resource_address() == self.banker_ticket_address);

            let data: BankerTicketData = ticket.non_fungible().data();

            assert!(Runtime::current_epoch() >= data.deposit_epoch + 500);
            assert!((data.bank_amount - amount) / dec!("1.5") >= data.borrow_amount);

            let resource_manager: &ResourceManager =
                borrow_resource_manager!(self.banker_ticket_address);

            self.bankers_auth_badge.authorize(|| {
                resource_manager.update_non_fungible_data(
                    &ticket.non_fungible::<BankerTicketData>().id(),
                    BankerTicketData {
                        bank_amount: data.bank_amount - amount,
                        borrow_amount: data.borrow_amount,
                        deposit_epoch: data.deposit_epoch,
                    },
                )
            });

            self.bank_pool.take(amount)
        }

        pub fn payback_loan(&mut self, ticket: Proof, payback: Bucket) {
            assert!(
                ticket.resource_address() == self.banker_ticket_address,
                " Your ticket resource address must match the banker ticket resource address"
            );

            let data: BankerTicketData = ticket.non_fungible().data();

            assert!(
                payback.amount() >= data.borrow_amount,
                "The payback amount must be greater or equal to your existing borrow amount"
            );

            let resource_manager: &ResourceManager =
                borrow_resource_manager!(self.banker_ticket_address);

            self.bankers_auth_badge.authorize(|| {
                resource_manager.update_non_fungible_data(
                    &ticket.non_fungible::<BankerTicketData>().id(),
                    BankerTicketData {
                        bank_amount: data.bank_amount,
                        borrow_amount: data.borrow_amount - payback.amount(),
                        deposit_epoch: data.deposit_epoch,
                    },
                )
            });

            self.bank_pool.put(payback);
        }

        pub fn claim_rewards(&mut self, ticket: Proof) -> Bucket {
            assert!(ticket.resource_address() == self.banker_ticket_address);

            let data: BankerTicketData = ticket.non_fungible().data();

            self.bankers_rewards
                .get_mut(&ticket.non_fungible::<BankerTicketData>().id())
                .unwrap()
                .take_all()
        }
        
        pub fn borrow(&mut self, ticket: Proof, amount: Decimal) -> Bucket {
            assert!(ticket.resource_address() == self.banker_ticket_address);

            let data: BankerTicketData = ticket.non_fungible().data();
            assert!(data.max_borrow_amount() >= amount);

            let commission_rate: Decimal = dec!("0.02");
            let party_commission: Decimal = amount * commission_rate;
            let mut borrowed_funds: Bucket = self.bank_pool.take(amount);
            let mut bankers_commission: Bucket = borrowed_funds.take(party_commission);
            
            let resource_manager: &ResourceManager =
                borrow_resource_manager!(self.banker_ticket_address);

            self.bankers_auth_badge.authorize(|| {
                resource_manager.update_non_fungible_data(
                    &ticket.non_fungible::<BankerTicketData>().id(),
                    BankerTicketData {
                        bank_amount: data.bank_amount,
                        borrow_amount: data.borrow_amount + amount,
                        deposit_epoch: data.deposit_epoch,
                    },
                )
            });

            for (non_fungible_id, vault) in &mut self.bankers_rewards {
                let data: BankerTicketData =
                    resource_manager.get_non_fungible_data(non_fungible_id);
                let amount_owed: Decimal =
                    data.bank_amount * data.percentage_of_unused_collateral() * party_commission
                        / self.bank_pool.amount();

                vault.put(bankers_commission.take(amount_owed));
            }
            borrowed_funds.put(bankers_commission);
            borrowed_funds
        }
    }
}