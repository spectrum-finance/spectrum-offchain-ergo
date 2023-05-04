use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use lazy_static::lazy_static;

const POOL_VALIDATOR_BYTES: &str =
    "19c0062804000400040204020404040404060406040804080404040204000400040204020400040a05000500040404\
    0204020e200508f3623d4b2be3bdb9737b3e65644f011167eefb830d9965205f022ceda40d040004020500040204020\
    4060500050005feffffffffffffffff01050005000402060101050005000100d81fd601b2a5730000d602db63087201\
    d603db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b2720\
    3730300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb2720273\
    0800d610b27203730900d6118c721001d6128c720b02d613998c720a027212d6148c720902d615b27205730a00d6169\
    a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab27205730f00\
    d61b7e721a05d61c9d7206721bd61d998c720c028c720d02d61e8c721002d61f998c720f02721ed1ededededed93b27\
    2027310007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201018cc7a70193\
    c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b172027311959\
    172137312d802d6209c721399721ba273137e721905d621b2a5731400ededed929a7e9972067214067e7207067e9c7e\
    9995907219721a72199a721a7315731605721c06937213f0721d937220f0721fedededed93cbc272217317938602720\
    e7213b2db6308722173180093860272117220b2db63087221731900e6c67221060893e4c67221070e8c720401958f72\
    13731aededec929a7e9972067214067e7207067e9c7e9995907219721a72199a721a731b731c05721c0692a39a9a721\
    59c721a7217b27205731d0093721df0721392721f95917219721a731e9c721d99721ba2731f7e721905d804d620e4c6\
    72010704d62199721a7220d6227e722105d62399997320721e9c7212722295ed917223732191721f7322edededed907\
    2209972197323909972149c7222721c9a721c7207907ef0998c7208027214069d9c99997e7214069d9c7e7206067e72\
    21067e721a0673247e721f067e722306937213732593721d73267327";

const BUNDLE_VALIDATOR_BYTES: &str =
    "19bc04210400040004040404040004020601010601000400050004020404040205feffffffffffffffff0104080400\
    04020502040405020402040004000101010005000404040004060404040205fcffffffffffffffff010100d80dd601b\
    2a5730000d602db63087201d603e4c6a7070ed604b2a4730100d605db63087204d6068cb2720573020002d607998cb2\
    7202730300027206d608e4c6a70608d609db6308a7d60ab27209730400d60bb27205730500d60c7306d60d7307d1ed9\
    38cb27202730800017203959372077309d80cd60eb2a5e4e3000400d60fb2a5e4e3010400d610b2e4c672040410730a\
    00d611c672010804d61299721095e67211e47211e4c672010704d6138cb27209730b0001d614db6308720fd615b2720\
    9730c00d6168c721502d6177e721205d6189972169c72178c720a02d6199d9c99997e8c720b02069d9c7ee4c6720405\
    05067e7212067e721006720c7e7218067e9999730d8cb27205730e00029c7206721706eded93c2720ed07208ededede\
    d93e4c6720f0608720893e4c6720f070e720393c2720fc2a795917212730fd801d61ab27214731000eded9386027213\
    7311b27214731200938c721a018c721501939972168c721a02721893860272137313b2721473140093b272147315007\
    20a95917219720dd801d61ab2db6308720e731600ed938c721a018c720b01927e8c721a0206997219720c9593721972\
    0d73177318958f7207731993b2db6308b2a4731a00731b0086029593b17209731c8cb27209731d00018cb27209731e0\
    001731f7320";

const DEPOSIT_TEMPLATE_BYTES: &str =
    "d808d601b2a4730000d602db63087201d6037301d604b2a5730200d6057303d606c57201d607b2a5730400d6088cb2\
    db6308a773050002eb027306d1ededed938cb27202730700017203ed93c27204720593860272067308b2db630872047\
    30900ededededed93cbc27207730a93d0e4c672070608720593e4c67207070e72039386028cb27202730b00017208b2\
    db63087207730c009386028cb27202730d00019c72087e730e05b2db63087207730f0093860272067310b2db6308720\
    773110090b0ada5d90109639593c272097312c1720973137314d90109599a8c7209018c7209027315";

pub const REDEEM_VALIDATOR_BYTES: &str =
    "19ad020a040208cd02217daf90deb73bdf8b6709bb42093fdfaff6573fd47b630e2d3fdd4a8193a74d0e2001010101\
    010101010101010101010101010101010101010101010101010101010e2000000000000000000000000000000000000\
    0000000000000000000000000000005d00f04000e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a0\
    6295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a\
    38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a573040500050005a09c01d801d601b2a57300\
    00eb027301d1eded93c27201730293860273037304b2db6308720173050090b0ada5d90102639593c272027306c1720\
    273077308d90102599a8c7202018c7202027309";

const REDEEM_TEMPLATE_BYTES: &str =
    "d801d601b2a5730000eb027301d1eded93c27201730293860273037304b2db6308720173050090b0ada5d901026395\
    93c272027306c1720273077308d90102599a8c7202018c7202027309";

lazy_static! {
    pub static ref POOL_VALIDATOR: ErgoTree =
        ErgoTree::sigma_parse_bytes(&base16::decode(POOL_VALIDATOR_BYTES.as_bytes()).unwrap()).unwrap();
    pub static ref BUNDLE_VALIDATOR: ErgoTree =
        ErgoTree::sigma_parse_bytes(&base16::decode(BUNDLE_VALIDATOR_BYTES.as_bytes()).unwrap()).unwrap();
    pub static ref REDEEM_VALIDATOR: ErgoTree =
        ErgoTree::sigma_parse_bytes(&base16::decode(REDEEM_VALIDATOR_BYTES.as_bytes()).unwrap()).unwrap();
    pub static ref DEPOSIT_TEMPLATE: Vec<u8> = base16::decode(DEPOSIT_TEMPLATE_BYTES.as_bytes()).unwrap();
    pub static ref REDEEM_TEMPLATE: Vec<u8> = base16::decode(REDEEM_TEMPLATE_BYTES.as_bytes()).unwrap();
}
