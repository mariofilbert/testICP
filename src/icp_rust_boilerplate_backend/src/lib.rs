#[ic_cdk::query]
fn greet(name: String) -> String {
    format!("Hello, {}!", name)
}

#[macro_use]
extern crate serde;
use candid::{Decode, Encode};
use ic_cdk::api::time;
use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};
use ic_stable_structures::{BoundedStorable, Cell, DefaultMemoryImpl, StableBTreeMap, Storable};
use std::{borrow::Cow, cell::RefCell};

type Memory = VirtualMemory<DefaultMemoryImpl>;
type IdCell = Cell<u64, Memory>;

#[derive(candid::CandidType, Clone, Serialize, Deserialize, Default)]
struct Warehouse {
    id: u64,
    name: String,
    created_at: u64,
}

#[derive(candid::CandidType, Clone, Serialize, Deserialize, Default)]
struct StockItem {
    item_id: u64,
    warehouse_id: u64,
    item_name: String,
    quantity: u64,
    created_at: u64,
    updated_at: Option<u64>,
}

impl Storable for Warehouse {
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }
}

impl BoundedStorable for Warehouse {
    const MAX_SIZE: u32 = 1024;
    const IS_FIXED_SIZE: bool = false;
}

impl Storable for StockItem {
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }
}

impl BoundedStorable for StockItem {
    const MAX_SIZE: u32 = 1024;
    const IS_FIXED_SIZE: bool = false;
}

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> = RefCell::new(
        MemoryManager::init(DefaultMemoryImpl::default())
    );

    static WAREHOUSE_ID_COUNTER: RefCell<IdCell> = RefCell::new(
        IdCell::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0))), 0)
            .expect("Cannot create warehouse id counter")
    );

    static ITEM_ID_COUNTER: RefCell<IdCell> = RefCell::new(
        IdCell::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1))), 0)
            .expect("Cannot create item id counter")
    );

    static WAREHOUSE_STORAGE: RefCell<StableBTreeMap<u64, Warehouse, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2)))
    ));

    static STOCK_STORAGE: RefCell<StableBTreeMap<u64, StockItem, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(3)))
    ));
}

#[derive(candid::CandidType, Serialize, Deserialize, Default)]
struct WarehousePayload {
    name: String,
}

#[derive(candid::CandidType, Serialize, Deserialize, Default)]
struct StockItemPayload {
    warehouse_id: u64,
    item_name: String,
    quantity: u64,
}

#[ic_cdk::query]
fn get_warehouse(id: u64) -> Result<Warehouse, Error> {
    match _get_warehouse(&id) {
        Some(warehouse) => Ok(warehouse),
        None => Err(Error::NotFound {
            msg: format!("A warehouse with id={} not found", id),
        }),
    }
}

#[ic_cdk::update]
fn add_warehouse(payload: WarehousePayload) -> Result<Warehouse, Error> {
    let id = WAREHOUSE_ID_COUNTER
        .with(|counter| {
            let current_value = *counter.borrow().get();
            counter.borrow_mut().set(current_value + 1)
        })
        .expect("cannot increment warehouse id counter");

    let warehouse = Warehouse {
        id,
        name: payload.name,
        created_at: time(),
    };

    WAREHOUSE_STORAGE.with(|storage| storage.borrow_mut().insert(id, warehouse.clone()));

    Ok(warehouse)
}

#[ic_cdk::update]
fn add_item_to_warehouse(payload: StockItemPayload) -> Result<StockItem, Error> {
    match WAREHOUSE_STORAGE.with(|storage| storage.borrow().get(&payload.warehouse_id)) {
        Some(_) => {
            let item_id = ITEM_ID_COUNTER
                .with(|counter| {
                    let current_value = *counter.borrow().get();
                    counter.borrow_mut().set(current_value + 1)
                })
                .expect("cannot increment item id counter");

            let item = StockItem {
                item_id,
                warehouse_id: payload.warehouse_id,
                item_name: payload.item_name,
                quantity: payload.quantity,
                created_at: time(),
                updated_at: None,
            };

            STOCK_STORAGE.with(|storage| storage.borrow_mut().insert(item_id, item.clone()));
            Ok(item)
        }
        None => Err(Error::NotFound {
            msg: format!("Warehouse with id={} not found", payload.warehouse_id),
        }),
    }
}

#[ic_cdk::query]
fn check_stock(item_id: u64) -> Result<StockItem, Error> {
    match STOCK_STORAGE.with(|storage| storage.borrow().get(&item_id)) {
        Some(stock_item) => Ok(stock_item),
        None => Err(Error::NotFound {
            msg: format!("Item with id={} not found", item_id),
        }),
    }
}

#[ic_cdk::update]
fn transfer_item(item_id: u64, to_warehouse_id: u64, quantity: u64) -> Result<(), Error> {
    STOCK_STORAGE.with(|storage| {
        if let Some(mut item) = storage.borrow_mut().get_mut(&item_id) {
            if item.quantity < quantity {
                return Err(Error::NotEnoughStock {
                    msg: format!(
                        "Not enough stock for item_id={}, available={}, requested={}",
                        item_id, item.quantity, quantity
                    ),
                });
            }

            item.quantity -= quantity;
            item.updated_at = Some(time());

            let new_item = StockItem {
                item_id: ITEM_ID_COUNTER.with(|counter| *counter.borrow().get()),
                warehouse_id: to_warehouse_id,
                item_name: item.item_name.clone(),
                quantity,
                created_at: time(),
                updated_at: None,
            };

            STOCK_STORAGE.with(|s| s.borrow_mut().insert(new_item.item_id, new_item));
            Ok(())
        } else {
            Err(Error::NotFound {
                msg: format!("Item with id={} not found", item_id),
            })
        }
    })
}

#[ic_cdk::query]
fn get_warehouse_stock(warehouse_id: u64) -> Vec<StockItem> {
    STOCK_STORAGE.with(|storage| {
        storage
            .borrow()
            .iter()
            .filter_map(|(_, item)| {
                if item.warehouse_id == warehouse_id {
                    Some(item)
                } else {
                    None
                }
            })
            .collect()
    })
}

#[derive(candid::CandidType, Deserialize, Serialize)]
enum Error {
    NotFound { msg: String },
    NotEnoughStock { msg: String },
}

// helper functions
fn _get_warehouse(id: &u64) -> Option<Warehouse> {
    WAREHOUSE_STORAGE.with(|service| service.borrow().get(id))
}

// need this to generate candid
ic_cdk::export_candid!();
